use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::{bench_public, indexer, mcp, output, query, store};

mod daemon;
use daemon::*;

const REHEARSAL_RETRIEVAL_FIXTURES: &str = "benchmarks/retrieval-fixtures.json";
const REHEARSAL_EXTERNAL_MANIFEST: &str = "benchmarks/external-github-manifest.example.json";
const OBSERVED_CODEX_OSS_50_MANIFEST: &str = "benchmarks/evidence/observed-codex-oss-50.local.json";
const REHEARSAL_REPORT_LIMIT: usize = 24;
const REHEARSAL_SNIPPETS_PER_FILE: usize = 2;
const MAX_LOCAL_EXPANSION_NEXT_FILES: usize = 1;

#[derive(Debug, Clone, Copy, Serialize)]
struct ExternalBenchmarkFixture {
    repo: &'static str,
    suite: &'static str,
    trace: &'static str,
}

const EXTERNAL_BENCHMARK_FIXTURES: &[ExternalBenchmarkFixture] = &[
    ExternalBenchmarkFixture {
        repo: "benchmarks/github-ripgrep",
        suite: "benchmarks/external-ripgrep-suite.json",
        trace: "benchmarks/external-ripgrep-trace.json",
    },
    ExternalBenchmarkFixture {
        repo: "benchmarks/github-fd",
        suite: "benchmarks/external-fd-suite.json",
        trace: "benchmarks/external-fd-trace.json",
    },
    ExternalBenchmarkFixture {
        repo: "benchmarks/github-axum",
        suite: "benchmarks/external-axum-suite.json",
        trace: "benchmarks/external-axum-trace.json",
    },
    ExternalBenchmarkFixture {
        repo: "benchmarks/github-flask",
        suite: "benchmarks/external-flask-suite.json",
        trace: "benchmarks/external-flask-trace.json",
    },
    ExternalBenchmarkFixture {
        repo: "benchmarks/github-black",
        suite: "benchmarks/external-black-suite.json",
        trace: "benchmarks/external-black-trace.json",
    },
    ExternalBenchmarkFixture {
        repo: "benchmarks/github-httpx",
        suite: "benchmarks/external-httpx-suite.json",
        trace: "benchmarks/external-httpx-trace.json",
    },
];

#[derive(Debug, Clone, Deserialize)]
struct CompetitiveReportManifest {
    #[serde(default = "default_competitive_goal")]
    goal: String,
    benchmark: query::BenchmarkReportManifest,
    #[serde(default)]
    natural_language_benchmark: Option<query::BenchmarkReportManifest>,
    #[serde(default)]
    performance: Option<CompetitivePerformanceManifest>,
    #[serde(default)]
    targets: CompetitiveReportTargets,
    #[serde(default)]
    competitors: Vec<CompetitiveCompetitorInput>,
}

fn default_competitive_goal() -> String {
    "Make CallSieve the default local-first context layer an agent uses before it searches, reads, edits, or reviews a repository.".to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct CompetitiveReportTargets {
    #[serde(default = "default_competitive_min_first_correct_file_rate")]
    minimum_first_correct_file_rate_at_k: f64,
    #[serde(default)]
    minimum_expected_file_recall: f64,
    #[serde(default = "default_competitive_min_natural_language_first_correct_file_rate")]
    minimum_natural_language_first_correct_file_rate_at_k: f64,
    #[serde(default)]
    minimum_natural_language_expected_file_recall: f64,
    #[serde(default)]
    maximum_first_query_p95_ms: Option<u64>,
    #[serde(default)]
    maximum_default_packet_tokens: Option<usize>,
    #[serde(default)]
    require_natural_language_benchmark: bool,
    #[serde(default)]
    require_trace_or_receipt_proof: bool,
    #[serde(default = "default_true")]
    require_zero_retrieval_model_tokens: bool,
    #[serde(default = "default_true")]
    require_zero_per_query_retrieval_cost: bool,
    #[serde(default = "default_true")]
    require_no_default_code_upload: bool,
    #[serde(default = "default_competitive_min_agent_coverage")]
    minimum_agent_coverage: usize,
    #[serde(default)]
    required_agents: Vec<String>,
}

impl Default for CompetitiveReportTargets {
    fn default() -> Self {
        Self {
            minimum_first_correct_file_rate_at_k: default_competitive_min_first_correct_file_rate(),
            minimum_expected_file_recall: 0.0,
            minimum_natural_language_first_correct_file_rate_at_k:
                default_competitive_min_natural_language_first_correct_file_rate(),
            minimum_natural_language_expected_file_recall: 0.0,
            maximum_first_query_p95_ms: None,
            maximum_default_packet_tokens: None,
            require_natural_language_benchmark: false,
            require_trace_or_receipt_proof: false,
            require_zero_retrieval_model_tokens: true,
            require_zero_per_query_retrieval_cost: true,
            require_no_default_code_upload: true,
            minimum_agent_coverage: default_competitive_min_agent_coverage(),
            required_agents: Vec::new(),
        }
    }
}

fn default_competitive_min_first_correct_file_rate() -> f64 {
    0.60
}

fn default_competitive_min_natural_language_first_correct_file_rate() -> f64 {
    0.45
}

fn default_competitive_min_agent_coverage() -> usize {
    12
}

#[derive(Debug, Clone, Deserialize)]
struct CompetitivePerformanceManifest {
    path: PathBuf,
    #[serde(default)]
    tasks: Option<PathBuf>,
    #[serde(default = "default_competitive_perf_iterations")]
    iterations: usize,
}

fn default_competitive_perf_iterations() -> usize {
    3
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CompetitiveCompetitorInput {
    name: String,
    #[serde(default, alias = "class")]
    category: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    install_friction: String,
    #[serde(default)]
    first_query_time: String,
    #[serde(default)]
    code_upload_requirement: String,
    #[serde(default)]
    retrieval_model_token_cost: String,
    #[serde(default)]
    per_query_retrieval_cost: String,
    #[serde(default)]
    packet_compactness: String,
    #[serde(default)]
    explainability: String,
    #[serde(default)]
    agent_coverage: String,
    #[serde(default)]
    trace_or_receipt_proof: String,
    #[serde(default)]
    where_callsieve_should_win: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicProofManifest {
    #[serde(default = "default_public_proof_goal")]
    goal: String,
    competitive_report: PathBuf,
    #[serde(default = "default_public_repos_dir")]
    public_repos_dir: PathBuf,
    #[serde(default)]
    miss_triage: Option<PathBuf>,
    #[serde(default)]
    targets: PublicProofTargets,
    #[serde(default)]
    public_reports: Vec<PublicBenchmarkReportInput>,
    #[serde(default)]
    public_result_catalog: Vec<PathBuf>,
    #[serde(default)]
    repo_packer_baselines: Vec<PublicRepoPackerBaselineInput>,
    #[serde(default)]
    agent_native_search_baselines: Vec<PublicAgentNativeSearchBaselineInput>,
    #[serde(default)]
    evidence_commands: Vec<String>,
    #[serde(default)]
    terminal_artifacts: Vec<PublicProofArtifactInput>,
}

fn default_public_proof_goal() -> String {
    "Make CallSieve win on external, credible retrieval proof while preserving local-first zero-token retrieval.".to_string()
}

fn default_public_repos_dir() -> PathBuf {
    PathBuf::from("benchmarks/public/repos")
}

#[derive(Debug, Clone, Deserialize)]
struct PublicProofTargets {
    #[serde(default = "default_true")]
    require_competitive_report_pass: bool,
    #[serde(default = "default_true")]
    require_zero_retrieval_model_tokens: bool,
    #[serde(default = "default_true")]
    require_zero_per_query_retrieval_cost: bool,
    #[serde(default = "default_true")]
    require_no_default_code_upload: bool,
    #[serde(default)]
    maximum_default_packet_tokens: Option<usize>,
    #[serde(default = "default_true")]
    require_broad_read_payload_reduction: bool,
    #[serde(default = "default_true")]
    require_context_packet_quality: bool,
    #[serde(default)]
    require_current_public_report_retrieval_contract: bool,
    #[serde(default)]
    minimum_broad_read_context_payload_reduction_percent: f64,
    #[serde(default = "default_public_proof_min_reports")]
    minimum_public_reports: usize,
    #[serde(default)]
    minimum_total_public_evaluated: Option<usize>,
    #[serde(default)]
    minimum_agent_native_measured_baselines: Option<usize>,
    #[serde(default)]
    minimum_agent_native_distinct_tools: Option<usize>,
    #[serde(default)]
    minimum_agent_native_repositories: Option<usize>,
    #[serde(default)]
    minimum_agent_native_task_languages: Option<usize>,
    #[serde(default = "default_public_proof_swe_bench_style_goal")]
    target_swe_bench_style_first_correct_file_rate_at_k: f64,
    #[serde(default = "default_public_proof_natural_language_goal")]
    target_natural_language_first_correct_file_rate_at_k: f64,
}

impl Default for PublicProofTargets {
    fn default() -> Self {
        Self {
            require_competitive_report_pass: true,
            require_zero_retrieval_model_tokens: true,
            require_zero_per_query_retrieval_cost: true,
            require_no_default_code_upload: true,
            maximum_default_packet_tokens: None,
            require_broad_read_payload_reduction: true,
            require_context_packet_quality: true,
            require_current_public_report_retrieval_contract: false,
            minimum_broad_read_context_payload_reduction_percent: 0.0,
            minimum_public_reports: default_public_proof_min_reports(),
            minimum_total_public_evaluated: None,
            minimum_agent_native_measured_baselines: None,
            minimum_agent_native_distinct_tools: None,
            minimum_agent_native_repositories: None,
            minimum_agent_native_task_languages: None,
            target_swe_bench_style_first_correct_file_rate_at_k:
                default_public_proof_swe_bench_style_goal(),
            target_natural_language_first_correct_file_rate_at_k:
                default_public_proof_natural_language_goal(),
        }
    }
}

fn default_public_proof_min_reports() -> usize {
    1
}

fn default_public_proof_swe_bench_style_goal() -> f64 {
    0.62
}

fn default_public_proof_natural_language_goal() -> f64 {
    0.45
}

#[derive(Debug, Clone, Deserialize)]
struct PublicBenchmarkReportInput {
    id: String,
    path: PathBuf,
    #[serde(default)]
    category: String,
    #[serde(default = "default_public_proof_preferred_arm")]
    preferred_arm: String,
    #[serde(default)]
    minimum_first_correct_file_rate_at_k: Option<f64>,
    #[serde(default)]
    minimum_minus_grep: Option<f64>,
    #[serde(default)]
    require_grep_baseline: bool,
    #[serde(default)]
    minimum_evaluated: Option<usize>,
    #[serde(default)]
    require_current_retrieval_contract: Option<bool>,
}

fn default_public_proof_preferred_arm() -> String {
    "lexical".to_string()
}

#[derive(Debug, Clone, Deserialize)]
struct PublicRepoPackerBaselineInput {
    id: String,
    #[serde(default)]
    tool: String,
    path: PathBuf,
    #[serde(default)]
    command: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    token_count_pointer: Option<String>,
    #[serde(default)]
    byte_count_pointer: Option<String>,
    #[serde(default)]
    file_count_pointer: Option<String>,
    #[serde(default)]
    minimum_token_ratio_vs_callsieve_packet: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicAgentNativeSearchBaselineInput {
    id: String,
    #[serde(default)]
    tool: String,
    path: PathBuf,
    #[serde(default)]
    command: String,
    #[serde(default)]
    required: bool,
    #[serde(default = "default_agent_native_locally_measured_pointer")]
    locally_measured_pointer: String,
    #[serde(default = "default_agent_native_task_count_pointer")]
    task_count_pointer: String,
    #[serde(default = "default_agent_native_agent_rate_pointer")]
    agent_native_first_correct_file_rate_at_k_pointer: String,
    #[serde(default = "default_agent_native_callsieve_rate_pointer")]
    callsieve_first_correct_file_rate_at_k_pointer: String,
    #[serde(default = "default_agent_native_context_tokens_pointer")]
    agent_native_average_context_tokens_pointer: String,
    #[serde(default = "default_agent_native_callsieve_packet_pointer")]
    callsieve_average_packet_tokens_pointer: String,
    #[serde(default)]
    minimum_tasks: Option<usize>,
    #[serde(default)]
    minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k: Option<f64>,
    #[serde(default)]
    minimum_agent_native_context_token_ratio_vs_callsieve: Option<f64>,
}

fn default_agent_native_locally_measured_pointer() -> String {
    "/locally_measured".to_string()
}

fn default_agent_native_task_count_pointer() -> String {
    "/metrics/task_count".to_string()
}

fn default_agent_native_agent_rate_pointer() -> String {
    "/metrics/agent_native_first_correct_file_rate_at_k".to_string()
}

fn default_agent_native_callsieve_rate_pointer() -> String {
    "/metrics/callsieve_first_correct_file_rate_at_k".to_string()
}

fn default_agent_native_context_tokens_pointer() -> String {
    "/metrics/agent_native_average_context_tokens".to_string()
}

fn default_agent_native_callsieve_packet_pointer() -> String {
    "/metrics/callsieve_average_packet_tokens".to_string()
}

#[derive(Debug, Serialize)]
struct RepoPackBaselineOutput {
    command: &'static str,
    schema_version: u32,
    id: String,
    tool: String,
    generated_at: u64,
    root: String,
    inclusion_scope: &'static str,
    token_estimate: &'static str,
    locally_measured: bool,
    measurement_notes: Vec<&'static str>,
    metrics: RepoPackBaselineMetrics,
    largest_files: Vec<RepoPackBaselineFile>,
}

#[derive(Debug, Serialize)]
struct RepoPackBaselineMetrics {
    file_count: usize,
    source_bytes: usize,
    framing_bytes: usize,
    total_bytes: usize,
    total_tokens: usize,
}

#[derive(Debug, Clone, Serialize)]
struct RepoPackBaselineFile {
    path: String,
    bytes: usize,
    tokens: usize,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AgentNativeBaselineInput {
    Tasks(Vec<AgentNativeBaselineTaskInput>),
    Object {
        tasks: Vec<AgentNativeBaselineTaskInput>,
    },
}

impl AgentNativeBaselineInput {
    fn into_tasks(self) -> Vec<AgentNativeBaselineTaskInput> {
        match self {
            AgentNativeBaselineInput::Tasks(tasks) | AgentNativeBaselineInput::Object { tasks } => {
                tasks
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct AgentNativeBaselineTaskInput {
    id: String,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    base_commit: Option<String>,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    expected_files: Vec<String>,
    #[serde(default)]
    agent_native_files: Vec<String>,
    #[serde(default)]
    callsieve_files: Vec<String>,
    #[serde(default)]
    agent_native_context_tokens: usize,
    #[serde(default)]
    callsieve_packet_tokens: usize,
    #[serde(default)]
    callsieve_index_fingerprint: Option<String>,
    #[serde(default)]
    recording_status: Option<String>,
}

#[derive(Debug, Serialize)]
struct AgentNativeBaselineOutput {
    command: &'static str,
    schema_version: u32,
    id: String,
    tool: String,
    generated_at: u64,
    locally_measured: bool,
    measurement_scope: &'static str,
    measurement_command: String,
    source_artifacts: Vec<String>,
    transcript_provenance_status: &'static str,
    source_artifact_evidence: Vec<AgentNativeSourceArtifactEvidence>,
    measurement_notes: Vec<String>,
    k: usize,
    metrics: AgentNativeBaselineMetrics,
    tasks: Vec<AgentNativeBaselineTaskEvidence>,
}

#[derive(Debug, Clone, Serialize)]
struct AgentNativeSourceArtifactEvidence {
    path: String,
    role: &'static str,
    bytes: usize,
    hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentNativeCheckMode {
    Template,
    Measured,
}

#[derive(Debug, Serialize)]
struct AgentNativeCheckOutput {
    command: &'static str,
    schema_version: u32,
    status: String,
    mode: AgentNativeCheckMode,
    tasks_path: String,
    task_count: usize,
    tasks_with_expected_files: usize,
    tasks_with_callsieve_files: usize,
    tasks_with_callsieve_packet_tokens: usize,
    tasks_with_agent_native_files: usize,
    tasks_with_agent_native_context_tokens: usize,
    tasks_ready_for_mode: usize,
    source_artifact_evidence: Vec<AgentNativeSourceArtifactEvidence>,
    issues: Vec<AgentNativeCheckIssue>,
    next_step: String,
}

#[derive(Debug, Serialize)]
struct AgentNativeCheckIssue {
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<String>,
    field: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct AgentNativeBaselineMetrics {
    task_count: usize,
    agent_native_first_correct_file_hits: usize,
    callsieve_first_correct_file_hits: usize,
    agent_native_first_correct_file_rate_at_k: f64,
    callsieve_first_correct_file_rate_at_k: f64,
    callsieve_minus_agent_native_first_correct_file_rate_at_k: f64,
    agent_native_total_context_tokens: usize,
    callsieve_total_packet_tokens: usize,
    agent_native_average_context_tokens: usize,
    callsieve_average_packet_tokens: usize,
    agent_native_context_token_ratio_vs_callsieve: f64,
}

#[derive(Debug, Serialize)]
struct AgentNativeBaselineTaskEvidence {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<String>,
    expected_files: Vec<String>,
    agent_native_files: Vec<String>,
    callsieve_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    callsieve_index_fingerprint: Option<String>,
    agent_native_first_correct_file_at_k: bool,
    callsieve_first_correct_file_at_k: bool,
    agent_native_context_tokens: usize,
    callsieve_packet_tokens: usize,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AgentNativeTemplateInput {
    Tasks(Vec<AgentNativeTemplateTaskInput>),
    Object {
        tasks: Vec<AgentNativeTemplateTaskInput>,
    },
    PublicIssues {
        issues: Vec<AgentNativeTemplateIssueInput>,
    },
}

impl AgentNativeTemplateInput {
    fn into_tasks(self) -> Vec<AgentNativeTemplateTaskInput> {
        match self {
            AgentNativeTemplateInput::Tasks(tasks) | AgentNativeTemplateInput::Object { tasks } => {
                tasks
            }
            AgentNativeTemplateInput::PublicIssues { issues } => issues
                .into_iter()
                .map(|issue| AgentNativeTemplateTaskInput {
                    id: issue.id,
                    repo: issue.repo,
                    base_commit: issue.base_commit,
                    task: issue.task,
                    expected_files: issue.ground_truth_files,
                    critical_files: Vec::new(),
                })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct AgentNativeTemplateTaskInput {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    base_commit: Option<String>,
    task: String,
    #[serde(default)]
    expected_files: Vec<String>,
    #[serde(default)]
    critical_files: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AgentNativeTemplateIssueInput {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    base_commit: Option<String>,
    task: String,
    #[serde(default)]
    ground_truth_files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AgentNativeTemplateOutput {
    command: &'static str,
    schema_version: u32,
    root: String,
    pinned_base_commits: bool,
    index_fingerprint: String,
    retrieval_contract_fingerprint: String,
    source_tasks: String,
    source_tasks_hash: String,
    locally_measured: bool,
    status: &'static str,
    k: usize,
    limit: usize,
    snippets_per_file: usize,
    profile: &'static str,
    token_budget: usize,
    instructions: Vec<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    tasks: Vec<AgentNativeTemplateTaskOutput>,
}

#[derive(Debug, Clone, Copy)]
struct AgentNativeTemplateOptions {
    k: usize,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
    profile: ContextProfileArg,
    token_budget: usize,
}

#[derive(Debug, Serialize)]
struct AgentNativeTemplateTaskOutput {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_commit: Option<String>,
    task: String,
    expected_files: Vec<String>,
    agent_native_files: Vec<String>,
    callsieve_files: Vec<String>,
    agent_native_context_tokens: usize,
    callsieve_packet_tokens: usize,
    callsieve_index_fingerprint: String,
    callsieve_first_correct_file_at_k: bool,
    recording_status: &'static str,
}

#[derive(Debug, Clone, Deserialize)]
struct PublicProofArtifactInput {
    id: String,
    path: PathBuf,
    #[serde(default = "default_public_proof_artifact_kind")]
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    required: bool,
}

fn default_public_proof_artifact_kind() -> String {
    "terminal_output".to_string()
}

#[derive(Debug, Parser)]
#[command(
    name = "callsieve",
    version,
    about = "Local-first codebase retrieval for AI coding agents"
)]
pub struct Cli {
    #[arg(long, global = true)]
    verbose: bool,

    #[arg(long, global = true)]
    pretty: bool,

    /// Tokenizer for budget enforcement and proof token counts:
    /// `heuristic` (default, dependency-light bytes/4), or `o200k` / `cl100k`
    /// for exact counts (requires a binary built with `--features tokenizers`).
    #[arg(long, global = true, default_value = "heuristic")]
    tokenizer: String,

    /// Enable BM25+ length normalization for the content-keyword ranking signal.
    /// Opt-in and deterministic; off by default so default ranking is unchanged.
    #[arg(long, global = true)]
    bm25: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build or replace the local CallSieve index.
    Index {
        path: PathBuf,

        /// Accepted for CLI stability. The current index command always rebuilds.
        #[arg(long)]
        refresh: bool,

        /// Enrich reference edges with installed Language Server Protocol servers.
        #[arg(long)]
        lsp: bool,

        /// Build the optional local embeddings cache after indexing.
        #[arg(long)]
        embeddings: bool,

        /// Embedding model for the optional cache (requires --embeddings).
        #[arg(long, value_enum, default_value_t = EmbedModelArg::Small)]
        embed_model: EmbedModelArg,
    },

    /// List indexed symbols.
    Symbols {
        path: PathBuf,

        #[arg(long, default_value_t = 100)]
        limit: usize,
    },

    /// Find indexed symbols by name.
    Symbol {
        path: PathBuf,
        symbol_name: String,

        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Return targeted symbols, bounded snippets, graph edges, and related tests for one file.
    Focus {
        path: PathBuf,

        #[arg(long)]
        file: String,

        #[arg(long)]
        symbol: Option<String>,

        #[arg(long)]
        line: Option<usize>,

        #[arg(long)]
        references: bool,

        #[arg(long, default_value_t = 1)]
        snippets_per_symbol: usize,

        /// Collapse symbol bodies to signature + `{ … }` markers for a compact,
        /// low-token view of the file's shape.
        #[arg(long)]
        skeleton: bool,
    },

    /// Return import, caller, callee, and blast-radius hints for one indexed file.
    Related {
        path: PathBuf,

        #[arg(long)]
        file: String,
    },

    /// Return tests likely related to one indexed file.
    Tests {
        path: PathBuf,

        #[arg(long)]
        file: String,
    },

    /// Rank indexed files and symbols for a natural-language question.
    Query {
        path: PathBuf,
        question: String,

        #[arg(long, default_value_t = 10)]
        limit: usize,

        #[arg(long)]
        no_snippets: bool,

        /// Include structured scoring components for ranking diagnostics.
        #[arg(long)]
        why_debug: bool,
    },

    /// Build a compact read-first packet for a coding task.
    Context {
        path: PathBuf,
        task: String,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,

        /// Include structured scoring components for ranking diagnostics.
        #[arg(long)]
        why_debug: bool,

        #[arg(long, value_enum, default_value_t = ContextProfileArg::Normal)]
        profile: ContextProfileArg,

        #[arg(long)]
        token_budget: Option<usize>,

        #[arg(long, value_enum, default_value_t = AgentOutputFormat::Json)]
        format: AgentOutputFormat,
    },

    /// Build an agent-ready context packet agents should request before grep.
    AgentContext {
        path: PathBuf,
        task: String,

        #[arg(long, default_value_t = query::DEFAULT_AGENT_CONTEXT_LIMIT)]
        limit: usize,

        #[arg(long, default_value_t = 0)]
        snippets_per_file: usize,

        /// Include structured scoring components for ranking diagnostics.
        #[arg(long)]
        why_debug: bool,

        #[arg(long, value_enum, default_value_t = ContextProfileArg::Skim)]
        profile: ContextProfileArg,

        #[arg(long, default_value_t = query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET)]
        token_budget: usize,

        #[arg(long, value_enum, default_value_t = AgentOutputFormat::Json)]
        format: AgentOutputFormat,

        /// Opt into local hybrid retrieval when built with --features embed.
        #[arg(long)]
        embeddings: bool,

        /// Path to a stack trace / error log; files it names are surfaced first.
        #[arg(long, value_name = "FILE")]
        error: Option<PathBuf>,

        /// Nudge recently-changed / hot files up the ranking using git history.
        #[arg(long)]
        git_boost: bool,

        /// Skip the running daemon's in-memory index and load directly.
        #[arg(long)]
        no_daemon: bool,

        /// Embedding model for hybrid retrieval (requires --embeddings and a
        /// cache built with the same model).
        #[arg(long, value_enum, default_value_t = EmbedModelArg::Small)]
        embed_model: EmbedModelArg,

        /// Boost files that observed agent sessions confirmed reading for
        /// similar tasks (local task memory).
        #[arg(long)]
        memory_boost: bool,
    },

    /// Build an index, return a sample context packet, and report context reduction.
    Demo {
        path: PathBuf,

        #[arg(long, default_value = "find where CLI commands are defined")]
        task: String,

        /// Enrich the demo index with installed Language Server Protocol servers.
        #[arg(long)]
        lsp: bool,
    },

    /// Clear the local task-memory hints used by agent-context.
    #[command(name = "memory-clear")]
    MemoryClear { path: PathBuf },

    /// Estimate token savings versus a naive grep/read loop.
    Benchmark {
        path: PathBuf,
        task: String,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,

        #[arg(long, value_enum, default_value_t = ContextProfileArg::Normal)]
        profile: ContextProfileArg,

        #[arg(long)]
        token_budget: Option<usize>,
    },

    /// Run benchmark estimates across a JSON task suite.
    BenchmarkSuite {
        path: PathBuf,
        tasks: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,

        #[arg(long, value_enum, default_value_t = ContextProfileArg::Normal)]
        profile: ContextProfileArg,

        #[arg(long)]
        token_budget: Option<usize>,
    },

    /// Evaluate read-first retrieval against expected and critical task fixtures.
    #[command(name = "eval-retrieval")]
    EvalRetrieval {
        manifest: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,

        #[arg(long, value_enum, default_value_t = ContextProfileArg::Normal)]
        profile: ContextProfileArg,

        #[arg(long)]
        token_budget: Option<usize>,

        /// Accepted for compatibility. CallSieve command output is JSON by default.
        #[arg(long)]
        json: bool,
    },

    /// Run fixed local context tasks and report p50/p95 latency.
    #[command(name = "perf-report")]
    PerfReport {
        path: PathBuf,

        #[arg(long)]
        tasks: Option<PathBuf>,

        #[arg(long, default_value_t = 5)]
        iterations: usize,

        /// Accepted for compatibility. CallSieve command output is JSON by default.
        #[arg(long)]
        json: bool,
    },

    /// Summarize observed baseline versus CallSieve agent session traces.
    TraceSummary { trace: PathBuf },

    /// Start a real observed Codex/ChatGPT session trace.
    SessionStart {
        path: PathBuf,
        task: String,

        #[arg(long, value_enum, default_value_t = AgentClient::Codex)]
        client: AgentClient,

        #[arg(long)]
        model: String,

        #[arg(long)]
        trace: PathBuf,

        #[arg(long = "expected-file")]
        expected_files: Vec<String>,

        #[arg(long = "critical-file")]
        critical_files: Vec<String>,
    },

    /// Append a command/read event to an observed session trace.
    SessionEvent {
        trace: PathBuf,

        #[arg(long = "command")]
        event_command: String,

        #[arg(long = "files-read")]
        files_read: Vec<String>,

        #[arg(
            long = "context-selected-file",
            alias = "context_selected_file",
            alias = "context-selected-files",
            alias = "context_selected_files"
        )]
        context_selected_files: Vec<String>,

        #[arg(long)]
        tokens: Option<usize>,

        #[arg(long, value_enum)]
        phase: Option<SessionPhase>,
    },

    /// Finish an observed session trace and write its summary JSON.
    SessionFinish {
        trace: PathBuf,

        #[arg(long)]
        out: PathBuf,

        /// Ground-truth patched file paths. Repeat for multiple files.
        #[arg(long = "ground-truth-files")]
        ground_truth_files: Vec<String>,
    },

    /// Generate a controlled baseline versus CallSieve trace from a benchmark suite.
    TraceReplay {
        path: PathBuf,
        tasks: PathBuf,
        output: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Check whether an observed trace used grep before CallSieve context.
    TraceCheck {
        trace: PathBuf,

        /// Also fail repeated file reads before CallSieve context.
        #[arg(long)]
        strict: bool,
    },

    /// Run benchmark suites across multiple local repositories from a manifest.
    BenchmarkReport {
        manifest: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Compare local CallSieve proof against competitor classes from a manifest.
    #[command(name = "competitive-report")]
    CompetitiveReport {
        manifest: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Combine competitive, public benchmark, and receipt evidence into a public proof artifact.
    #[command(name = "public-proof-report")]
    PublicProofReport {
        manifest: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Measure a safe local full-repo prompt-pack proxy for public proof baselines.
    #[command(name = "repo-pack-baseline")]
    RepoPackBaseline {
        path: PathBuf,

        #[arg(long, default_value = "full-repo-pack-proxy")]
        id: String,

        #[arg(long, default_value = "FullRepoPackProxy")]
        tool: String,

        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Build a standard artifact from measured agent-native search task results.
    #[command(name = "agent-native-baseline")]
    AgentNativeBaseline {
        tasks: PathBuf,

        #[arg(long, default_value = "agent-native-search")]
        id: String,

        #[arg(long, default_value = "AgentNativeSearch")]
        tool: String,

        #[arg(long, default_value_t = 5)]
        k: usize,

        #[arg(long, default_value = "")]
        measurement_command: String,

        #[arg(long = "source-artifact")]
        source_artifacts: Vec<PathBuf>,

        #[arg(long = "measurement-note")]
        measurement_notes: Vec<String>,

        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Validate an agent-native search task log before external measurement or baseline conversion.
    #[command(name = "agent-native-check")]
    AgentNativeCheck {
        tasks: PathBuf,

        #[arg(long, value_enum, default_value = "measured")]
        mode: AgentNativeCheckMode,

        #[arg(long = "source-artifact")]
        source_artifacts: Vec<PathBuf>,

        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Print or write the approved agent-native search measurement protocol.
    #[command(name = "agent-native-protocol")]
    AgentNativeProtocol {
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Pre-fill CallSieve's side of an agent-native search measurement task log.
    #[command(name = "agent-native-template")]
    AgentNativeTemplate {
        path: PathBuf,
        tasks: PathBuf,

        #[arg(long, default_value_t = 5)]
        k: usize,

        #[arg(long, default_value_t = query::DEFAULT_AGENT_CONTEXT_LIMIT)]
        limit: usize,

        #[arg(long, default_value_t = 0)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,

        #[arg(long, value_enum, default_value_t = ContextProfileArg::Skim)]
        profile: ContextProfileArg,

        #[arg(long, default_value_t = query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET)]
        token_budget: usize,

        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Validate a benchmark-report manifest before running evidence collection.
    BenchmarkDoctor { manifest: PathBuf },

    /// Run deterministic local proof rehearsal without claim proof.
    #[command(name = "proof-rehearsal")]
    ProofRehearsal {
        /// Only check local prerequisites and exit.
        #[arg(long)]
        preflight: bool,

        /// Apply safe local fixes: create evidence dirs, rebuild indexes, and fill missing traces.
        #[arg(long)]
        fix: bool,

        /// Skip already-passed ledger steps with matching signatures.
        #[arg(long)]
        resume: bool,

        /// Also run supplemental Ollama collection. This is not Codex proof.
        #[arg(long = "collect-ollama")]
        collect_ollama: bool,

        #[arg(
            long = "ollama-manifest",
            default_value = "benchmarks/evidence/observed-generic-ollama-100.local.json"
        )]
        ollama_manifest: PathBuf,

        #[arg(long = "ollama-model", default_value = "qwen2.5-coder:7b")]
        ollama_model: String,

        #[arg(long = "ollama-limit", default_value_t = 10)]
        ollama_limit: usize,

        #[arg(long = "ollama-context-limit", default_value_t = 24)]
        ollama_context_limit: usize,

        #[arg(long = "retry-count", default_value_t = 1)]
        retry_count: usize,

        #[arg(long, default_value = "benchmarks/evidence/rehearsal-run.local.json")]
        ledger: PathBuf,
    },

    /// Register the 50-session observed Codex OSS milestone manifest.
    #[command(name = "setup-observed-codex-oss-50")]
    SetupObservedCodexOss50 {
        #[arg(
            long,
            default_value = "benchmarks/evidence/observed-codex-oss-50.local.json"
        )]
        manifest: PathBuf,

        #[arg(long = "bootstrap-repos")]
        bootstrap_repos: bool,

        #[arg(long)]
        force: bool,

        #[arg(long = "skip-repo-check")]
        skip_repo_check: bool,
    },

    /// Register the 50-session observed Claude Code OSS milestone manifest.
    #[command(name = "setup-observed-claude-oss-50")]
    SetupObservedClaudeOss50 {
        #[arg(
            long,
            default_value = "benchmarks/evidence/observed-claude-oss-50.local.json"
        )]
        manifest: PathBuf,

        #[arg(long, default_value = "claude-opus-4-8")]
        model: String,

        #[arg(long = "bootstrap-repos")]
        bootstrap_repos: bool,

        #[arg(long)]
        force: bool,

        #[arg(long = "skip-repo-check")]
        skip_repo_check: bool,
    },

    /// Record one real observed paired-session event from any agent.
    #[command(name = "record-observed-session")]
    RecordObservedSession {
        #[arg(long)]
        manifest: PathBuf,

        #[arg(long, value_enum, default_value_t = AgentClient::Generic)]
        client: AgentClient,

        #[arg(long)]
        model: Option<String>,

        #[arg(long = "task-id", alias = "task_id")]
        task_id: String,

        #[arg(long, value_enum)]
        mode: PilotSessionMode,

        #[arg(long = "command")]
        event_command: String,

        #[arg(long)]
        tokens: Option<usize>,

        #[arg(long = "usage-json", alias = "usage_json")]
        usage_json: Option<PathBuf>,

        #[arg(long = "files-read", alias = "files_read")]
        files_read: Vec<String>,

        #[arg(
            long = "context-selected-file",
            alias = "context_selected_file",
            alias = "context-selected-files",
            alias = "context_selected_files"
        )]
        context_selected_files: Vec<String>,

        #[arg(long = "dry-run")]
        dry_run: bool,
    },

    /// Run Claude Code for one observed task, save stream JSON, and record it.
    #[command(name = "collect-claude-observed-session")]
    CollectClaudeObservedSession {
        #[arg(
            long,
            default_value = "benchmarks/evidence/observed-claude-oss-50.local.json"
        )]
        manifest: PathBuf,

        #[arg(long = "task-id", alias = "task_id")]
        task_id: String,

        #[arg(long, value_enum)]
        mode: PilotSessionMode,

        #[arg(long, default_value = "claude-opus-4-8")]
        model: String,

        #[arg(long = "max-budget-usd", default_value = "0.50")]
        max_budget_usd: String,

        #[arg(long)]
        artifact: Option<PathBuf>,

        #[arg(long = "context-limit", default_value_t = 4)]
        context_limit: usize,

        #[arg(long = "snippets-per-file", default_value_t = 1)]
        snippets_per_file: usize,

        #[arg(long = "allowed-tool")]
        allowed_tools: Vec<String>,

        #[arg(long = "dry-run")]
        dry_run: bool,
    },

    /// Run a buyer-facing observed proof sprint over existing pilot evidence tooling.
    #[command(name = "proof-sprint")]
    ProofSprint {
        #[command(subcommand)]
        command: ProofSprintCommand,
    },

    /// Record one real observed Codex paired-session event with transcript token counts.
    #[command(name = "record-codex-observed-session")]
    RecordCodexObservedSession {
        #[arg(
            long,
            default_value = "benchmarks/evidence/observed-codex-oss-50.local.json"
        )]
        manifest: PathBuf,

        #[arg(long = "task-id", alias = "task_id")]
        task_id: String,

        #[arg(long, value_enum)]
        mode: PilotSessionMode,

        #[arg(long = "command")]
        event_command: String,

        #[arg(long)]
        tokens: usize,

        #[arg(long = "files-read", alias = "files_read")]
        files_read: Vec<String>,

        #[arg(
            long = "context-selected-file",
            alias = "context_selected_file",
            alias = "context-selected-files",
            alias = "context_selected_files"
        )]
        context_selected_files: Vec<String>,

        #[arg(long = "dry-run")]
        dry_run: bool,
    },

    /// Create a 100-session observed pilot harness manifest.
    PilotInit {
        manifest: PathBuf,

        #[arg(long, default_value_t = 100)]
        sessions: usize,
    },

    /// Manage observed pilot harness tasks.
    #[command(name = "pilot-task")]
    PilotTask {
        #[command(subcommand)]
        command: PilotTaskCommand,
    },

    /// Append a measured baseline or CallSieve event to a pilot task.
    PilotRun {
        manifest: PathBuf,

        #[arg(long = "task-id")]
        task_id: String,

        #[arg(long, value_enum)]
        mode: PilotSessionMode,

        #[arg(long = "command")]
        event_command: String,

        #[arg(long = "files-read")]
        files_read: Vec<String>,

        #[arg(
            long = "context-selected-file",
            alias = "context_selected_file",
            alias = "context-selected-files",
            alias = "context_selected_files"
        )]
        context_selected_files: Vec<String>,

        #[arg(long)]
        tokens: usize,
    },

    /// Collect audited local Ollama paired sessions for pending pilot tasks.
    #[command(name = "pilot-collect-ollama")]
    PilotCollectOllama {
        manifest: PathBuf,

        #[arg(long, default_value = "qwen2.5-coder:7b")]
        model: String,

        #[arg(long, default_value_t = 100)]
        limit: usize,

        #[arg(long = "context-limit", default_value_t = 24)]
        context_limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long = "baseline-file-limit", default_value_t = 48)]
        baseline_file_limit: usize,

        #[arg(long = "baseline-line-limit", default_value_t = 240)]
        baseline_line_limit: usize,
    },

    /// Collect audited local LM Studio paired sessions for pending pilot tasks.
    #[command(name = "pilot-collect-lm-studio")]
    PilotCollectLmStudio {
        manifest: PathBuf,

        #[arg(long, default_value = "qwen3-coder-next")]
        model: String,

        #[arg(long = "base-url", default_value = "http://127.0.0.1:1234/v1")]
        base_url: String,

        #[arg(long, default_value_t = 100)]
        limit: usize,

        #[arg(long = "context-limit", default_value_t = 24)]
        context_limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long = "baseline-file-limit", default_value_t = 48)]
        baseline_file_limit: usize,

        #[arg(long = "baseline-line-limit", default_value_t = 240)]
        baseline_line_limit: usize,

        #[arg(long = "max-tokens", default_value_t = 512)]
        max_tokens: usize,
    },

    /// Validate paired observed pilot evidence before final proof generation.
    PilotQa { manifest: PathBuf },

    /// Write final proof manifest and proof-report JSON from pilot evidence.
    PilotFinalize {
        manifest: PathBuf,

        #[arg(long)]
        out: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Build a pilot proof artifact across local repos, suites, traces, status, and thresholds.
    PilotReport {
        manifest: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Build the top-level claim proof artifact with observed and controlled evidence separated.
    ProofReport {
        manifest: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Build the broad enterprise-proof artifact with strict observed-session gates.
    #[command(name = "enterprise-proof-report")]
    EnterpriseProofReport {
        manifest: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Validate a pilot-report manifest and local evidence prerequisites.
    PilotDoctor { manifest: PathBuf },

    /// Build a shareable external evidence packet from a pilot manifest.
    EvidencePack {
        manifest: PathBuf,

        #[arg(long)]
        anonymize: bool,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// CI-friendly before-grep policy check. Exits nonzero on violations.
    PolicyCheck {
        trace: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Run a minimal MCP stdio server exposing CallSieve tools.
    Mcp,

    /// Print a portable MCP server configuration for any AI CLI that supports MCP.
    #[command(name = "mcp-config")]
    McpConfig {
        path: PathBuf,

        #[arg(long, value_enum, default_value_t = McpConfigFormat::Json)]
        format: McpConfigFormat,
    },

    /// Print or write a local-first MCP Registry server.json descriptor.
    #[command(name = "mcp-registry-manifest")]
    McpRegistryManifest {
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Print or write the stable MCP callsieve_context structuredContent contract.
    #[command(name = "mcp-contract")]
    McpContract {
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Show index freshness, watch, schema, and LSP-enrichment status.
    Status { path: PathBuf },

    /// Run a portable local indexing daemon loop or one-shot daemon refresh.
    Daemon {
        path: PathBuf,

        #[arg(long)]
        lsp: bool,

        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,

        #[arg(long)]
        foreground: bool,

        #[arg(long)]
        background: bool,

        #[arg(long)]
        once: bool,
    },

    /// Show saved daemon health state.
    DaemonStatus { path: PathBuf },

    /// Request a running foreground daemon to stop.
    DaemonStop { path: PathBuf },

    /// Refresh once or continuously keep the local index fresh.
    Watch {
        path: PathBuf,

        #[arg(long, default_value_t = 1000)]
        debounce_ms: u64,

        #[arg(long)]
        foreground: bool,

        /// Enrich refreshed indexes with installed Language Server Protocol servers.
        #[arg(long)]
        lsp: bool,
    },

    /// Generate local agent config and rules that require CallSieve before grep.
    #[command(name = "agent-setup")]
    AgentSetup {
        path: PathBuf,

        #[arg(long, value_enum, default_value_t = AgentClient::Generic)]
        client: AgentClient,

        #[arg(long)]
        force: bool,
    },

    /// Build index, setup agent policy, start daemon, and optionally install strict grep shims.
    Bootstrap {
        path: PathBuf,

        #[arg(long, value_enum, default_value_t = AgentClient::Generic)]
        client: AgentClient,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        force: bool,

        #[arg(long)]
        lsp: bool,
    },

    /// Report or repair local CallSieve adoption checks for an agent client.
    Doctor {
        path: PathBuf,

        #[arg(long, value_enum, default_value_t = AgentClient::Generic)]
        client: AgentClient,

        #[arg(long)]
        fix: bool,

        #[arg(long)]
        strict: bool,
    },

    /// Generate local agent config and rules that require CallSieve before grep.
    #[command(name = "setup-agent")]
    SetupAgent {
        client: AgentClient,
        path: PathBuf,

        #[arg(long)]
        force: bool,
    },

    /// Detect installed agents on this machine and run setup for each, with no
    /// per-client decisions.
    #[command(name = "setup-auto")]
    SetupAuto {
        path: PathBuf,

        #[arg(long)]
        force: bool,

        /// Only report which agents would be configured, without writing files.
        #[arg(long)]
        dry_run: bool,
    },

    /// Write the local index to a portable file so teammates can start warm
    /// without re-indexing.
    #[command(name = "index-export")]
    IndexExport {
        path: PathBuf,

        #[arg(long)]
        out: PathBuf,
    },

    /// Install an exported index after verifying it matches this checkout by
    /// content hash. Matched files get local mtimes so freshness checks pass.
    #[command(name = "index-import")]
    IndexImport {
        path: PathBuf,

        #[arg(long)]
        from: PathBuf,

        /// Accept imports where up to 10% of files differ from this checkout;
        /// differing files stay stale and refresh on the next rebuild.
        #[arg(long)]
        allow_partial: bool,
    },

    /// Write this repo's learned task memory to a portable file so teammates
    /// (and other agents) inherit confirmed task-to-file associations.
    #[command(name = "memory-export")]
    MemoryExport {
        path: PathBuf,

        #[arg(long)]
        out: PathBuf,
    },

    /// Merge an exported task memory into this repo's store (newest entry
    /// per task wins; confirmed files union; 50-entry cap holds).
    #[command(name = "memory-import")]
    MemoryImport {
        path: PathBuf,

        #[arg(long)]
        from: PathBuf,
    },

    /// Emit a tamper-evident receipt for one observed agent session: packets
    /// served, context tokens, reads/greps, violations, edit impacts.
    Receipt {
        path: PathBuf,

        /// Session id; defaults to the most recent session trace.
        #[arg(long)]
        session: Option<String>,

        /// Use the most recent session trace (the default; accepted so the
        /// command the Stop hook prints works verbatim).
        #[arg(long, conflicts_with = "session")]
        latest: bool,

        #[arg(long, value_enum, default_value_t = ReceiptFormat::Json)]
        format: ReceiptFormat,
    },

    /// Aggregate receipts across every recorded session trace in the repo.
    Receipts { path: PathBuf },

    /// Generate Codex-first local bootstrap files without global PATH/profile mutation.
    CodexBootstrap {
        path: PathBuf,

        #[arg(long)]
        model: String,

        #[arg(long)]
        force: bool,
    },

    /// Install, inspect, or remove Codex lifecycle hooks.
    #[command(name = "codex-hooks")]
    CodexHooks {
        #[command(subcommand)]
        command: CodexHooksCommand,
    },

    /// Internal Codex lifecycle hook entrypoints.
    #[command(name = "codex-hook", hide = true)]
    CodexHook {
        #[command(subcommand)]
        command: CodexHookCommand,
    },

    /// Install, inspect, or remove Claude Code lifecycle hooks.
    #[command(name = "claude-hooks")]
    ClaudeHooks {
        #[command(subcommand)]
        command: ClaudeHooksCommand,
    },

    /// Internal Claude Code lifecycle hook entrypoints.
    #[command(name = "claude-hook", hide = true)]
    ClaudeHook {
        #[command(subcommand)]
        command: ClaudeHookCommand,
    },

    /// Install, inspect, or remove GitHub Copilot lifecycle hooks.
    #[command(name = "copilot-hooks")]
    CopilotHooks {
        #[command(subcommand)]
        command: ClientHooksCommand,
    },

    /// Internal GitHub Copilot lifecycle hook entrypoints.
    #[command(name = "copilot-hook", hide = true)]
    CopilotHook {
        #[command(subcommand)]
        command: ClientHookCommand,
    },

    /// Install, inspect, or remove OpenCode plugin hooks.
    #[command(name = "opencode-hooks")]
    OpenCodeHooks {
        #[command(subcommand)]
        command: ClientHooksCommand,
    },

    /// Internal OpenCode plugin hook entrypoints.
    #[command(name = "opencode-hook", hide = true)]
    OpenCodeHook {
        #[command(subcommand)]
        command: ClientHookCommand,
    },

    /// Install, inspect, or remove Antigravity CLI lifecycle hooks.
    #[command(name = "antigravity-hooks")]
    AntigravityHooks {
        #[command(subcommand)]
        command: ClientHooksCommand,
    },

    /// Internal Antigravity CLI lifecycle hook entrypoints.
    #[command(name = "antigravity-hook", hide = true)]
    AntigravityHook {
        #[command(subcommand)]
        command: ClientHookCommand,
    },

    /// Install, inspect, or remove Cline lifecycle hooks.
    #[command(name = "cline-hooks")]
    ClineHooks {
        #[command(subcommand)]
        command: ClientHooksCommand,
    },

    /// Internal Cline lifecycle hook entrypoints.
    #[command(name = "cline-hook", hide = true)]
    ClineHook {
        #[command(subcommand)]
        command: ClientHookCommand,
    },

    /// Generate project-local editor hooks that start the CallSieve daemon.
    EditorHook {
        path: PathBuf,

        #[arg(long, value_enum)]
        editor: EditorKind,

        #[arg(long)]
        force: bool,
    },

    /// Start a context-first guarded coding task and optionally write a trace stub.
    Guard {
        path: PathBuf,
        task: String,

        #[arg(long)]
        trace_out: Option<PathBuf>,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,
    },

    /// Start an agent task, return context, and optionally write the first trace event.
    Begin {
        path: PathBuf,
        task: String,

        #[arg(long, value_enum, default_value_t = AgentClient::Generic)]
        client: AgentClient,

        #[arg(long)]
        trace_out: Option<PathBuf>,

        #[arg(long)]
        proof_trace: bool,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,
    },

    /// Start a Codex/ChatGPT context-first session and write measurable trace JSON.
    CodexSession {
        path: PathBuf,
        task: String,

        #[arg(long)]
        trace_out: PathBuf,

        #[arg(long, default_value = "codex/chatgpt")]
        model: String,

        #[arg(long = "expected-file")]
        expected_files: Vec<String>,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Audit generated agent setup, index freshness, optional trace policy, and shim state.
    Enforce {
        path: PathBuf,

        #[arg(long, value_enum, default_value_t = AgentClient::Generic)]
        client: AgentClient,

        #[arg(long)]
        trace: Option<PathBuf>,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        require_shim: bool,
    },

    /// Install, inspect, or remove opt-in rg/grep PATH shims.
    Shim {
        #[command(subcommand)]
        command: ShimCommand,
    },

    /// Install, inspect, or remove repo-local hands-off agent hooks.
    Hook {
        #[command(subcommand)]
        command: HookCommand,
    },

    /// Return CallSieve context before optionally running rg.
    Grep {
        path: PathBuf,
        pattern: String,

        #[arg(long)]
        run_rg: bool,

        #[arg(long, default_value_t = query::DEFAULT_AGENT_CONTEXT_LIMIT)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long, hide = true)]
        shim_strict: bool,

        #[arg(long, hide = true)]
        shim_command: Option<String>,
    },

    /// Internal entrypoint used by generated grep shims.
    #[command(name = "shim-run", hide = true)]
    ShimRun {
        path: PathBuf,

        #[arg(long, value_enum)]
        tool: ShimTool,

        #[arg(long)]
        strict: bool,

        #[arg(last = true)]
        args: Vec<String>,
    },

    /// Show index statistics.
    Stats { path: PathBuf },

    /// Run the Mode A public benchmark: retrieval-only, offline, deterministic.
    #[command(name = "bench-public")]
    BenchPublic {
        /// Path to benchmarks/public/manifest.json.
        manifest: PathBuf,

        /// Directory containing cloned repos, one per <owner>/<repo> entry.
        repos_dir: PathBuf,

        /// K for first_correct_file_rate_at_k.
        #[arg(long)]
        k: Option<usize>,

        /// Where to write the aggregated results JSON.
        #[arg(long)]
        out: Option<PathBuf>,

        /// Opt into local hybrid retrieval when built with --features embed.
        #[arg(long)]
        embeddings: bool,

        /// Run lexical-vs-hybrid A/B comparison.
        #[arg(long)]
        compare: bool,
    },

    /// Clone pinned public benchmark repos and run retrieval evaluation.
    #[command(name = "bench-run")]
    BenchRun {
        /// Path to a public benchmark manifest.
        manifest: PathBuf,

        /// Work directory for benchmark clones under <owner>/<repo>.
        #[arg(long)]
        workdir: PathBuf,

        /// Run lexical-vs-hybrid A/B comparison.
        #[arg(long)]
        compare: bool,

        /// K for first_correct_file_rate_at_k.
        #[arg(long)]
        k: Option<usize>,

        /// Limit issues evaluated, useful for smoke runs.
        #[arg(long)]
        limit: Option<usize>,

        /// Where to write the results JSON.
        #[arg(long)]
        out: Option<PathBuf>,

        /// Reuse matching completed issue results from --out and update it after each issue.
        #[arg(long)]
        resume: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ShimCommand {
    /// Install local rg/grep wrappers under .callsieve/bin.
    Install {
        path: PathBuf,

        #[arg(long)]
        force: bool,

        #[arg(long)]
        strict: bool,
    },

    /// Verify shim files and PATH guidance.
    Doctor { path: PathBuf },

    /// Remove installed local rg/grep wrappers.
    Uninstall { path: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum HookCommand {
    /// Install repo-local launchers, policy files, daemon setup, and grep shims.
    Install {
        path: PathBuf,

        #[arg(long, value_enum, default_value_t = AgentClient::Generic)]
        client: AgentClient,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        force: bool,

        #[arg(long)]
        lsp: bool,
    },

    /// Verify repo-local hook files and PATH guidance.
    Doctor { path: PathBuf },

    /// Remove repo-local hook launchers and grep shims.
    Uninstall { path: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum CodexHooksCommand {
    /// Install repo-local Codex lifecycle hooks.
    Install {
        path: PathBuf,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        force: bool,

        #[arg(long, default_value_t = 6)]
        limit: usize,

        #[arg(long, default_value_t = 1)]
        snippets_per_file: usize,

        #[arg(long)]
        lsp: bool,
    },

    /// Verify repo-local Codex lifecycle hooks.
    Doctor {
        path: PathBuf,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        smoke: bool,

        #[arg(long)]
        fix: bool,
    },

    /// Record that a human reviewed and trusted the current Codex hook file.
    TrustAck { path: PathBuf },

    /// Remove generated repo-local Codex lifecycle hooks.
    Uninstall { path: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum CodexHookCommand {
    /// Handle Codex UserPromptSubmit hook JSON from stdin.
    #[command(name = "user-prompt-submit")]
    UserPromptSubmit {
        path: PathBuf,

        #[arg(long)]
        strict: bool,

        #[arg(long, default_value_t = 6)]
        limit: usize,

        #[arg(long, default_value_t = 1)]
        snippets_per_file: usize,
    },

    /// Handle Codex PreToolUse hook JSON from stdin.
    #[command(name = "pre-tool-use")]
    PreToolUse {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle Codex PostToolUse hook JSON from stdin.
    #[command(name = "post-tool-use")]
    PostToolUse {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle Codex PermissionRequest hook JSON from stdin.
    #[command(name = "permission-request")]
    PermissionRequest {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle Codex Stop hook JSON from stdin.
    Stop {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClaudeHooksCommand {
    /// Install repo-local Claude Code lifecycle hooks.
    Install {
        path: PathBuf,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        force: bool,

        #[arg(long, default_value_t = 6)]
        limit: usize,

        #[arg(long, default_value_t = 1)]
        snippets_per_file: usize,

        #[arg(long)]
        lsp: bool,
    },

    /// Verify repo-local Claude Code lifecycle hooks.
    Doctor {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Remove generated repo-local Claude Code lifecycle hooks.
    Uninstall { path: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum ClaudeHookCommand {
    /// Handle Claude Code UserPromptSubmit hook JSON from stdin.
    #[command(name = "user-prompt-submit")]
    UserPromptSubmit {
        path: PathBuf,

        #[arg(long)]
        strict: bool,

        #[arg(long, default_value_t = 6)]
        limit: usize,

        #[arg(long, default_value_t = 1)]
        snippets_per_file: usize,
    },

    /// Handle Claude Code PreToolUse hook JSON from stdin.
    #[command(name = "pre-tool-use")]
    PreToolUse {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle Claude Code PostToolUse hook JSON from stdin.
    #[command(name = "post-tool-use")]
    PostToolUse {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle Claude Code PermissionRequest hook JSON from stdin.
    #[command(name = "permission-request")]
    PermissionRequest {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle Claude Code Stop hook JSON from stdin.
    Stop {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClientHooksCommand {
    /// Install repo-local lifecycle hook or plugin files.
    Install {
        path: PathBuf,

        #[arg(long)]
        strict: bool,

        #[arg(long)]
        force: bool,

        #[arg(long, default_value_t = 6)]
        limit: usize,

        #[arg(long, default_value_t = 1)]
        snippets_per_file: usize,

        #[arg(long)]
        lsp: bool,
    },

    /// Verify repo-local lifecycle hook or plugin files.
    Doctor {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Remove generated repo-local lifecycle hook or plugin files.
    Uninstall { path: PathBuf },
}

#[derive(Debug, Subcommand)]
pub enum ClientHookCommand {
    /// Handle UserPromptSubmit or equivalent prompt hook JSON from stdin.
    #[command(name = "user-prompt-submit")]
    UserPromptSubmit {
        path: PathBuf,

        #[arg(long)]
        strict: bool,

        #[arg(long, default_value_t = 6)]
        limit: usize,

        #[arg(long, default_value_t = 1)]
        snippets_per_file: usize,
    },

    /// Handle PreToolUse or equivalent pre-tool hook JSON from stdin.
    #[command(name = "pre-tool-use")]
    PreToolUse {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle PostToolUse or equivalent post-tool hook JSON from stdin.
    #[command(name = "post-tool-use")]
    PostToolUse {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle PermissionRequest or equivalent permission hook JSON from stdin.
    #[command(name = "permission-request")]
    PermissionRequest {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },

    /// Handle Stop or equivalent session-complete hook JSON from stdin.
    Stop {
        path: PathBuf,

        #[arg(long)]
        strict: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ShimTool {
    Rg,
    Grep,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum PilotTaskCommand {
    /// Add a measured task to a pilot harness manifest.
    Add {
        manifest: PathBuf,
        repo: PathBuf,
        task: String,

        #[arg(long)]
        id: Option<String>,

        #[arg(long = "expected-file")]
        expected_files: Vec<String>,

        #[arg(long = "critical-file")]
        critical_files: Vec<String>,

        #[arg(long)]
        external: bool,

        #[arg(long, value_enum, default_value_t = AgentClient::Codex)]
        client: AgentClient,

        #[arg(long, default_value = "gpt-5-codex")]
        model: String,

        #[arg(long = "suite-path")]
        suite_path: Option<PathBuf>,

        #[arg(long = "pair-id")]
        pair_id: Option<String>,

        #[arg(long = "task-category", default_value = "code_change")]
        task_category: String,

        #[arg(long, default_value = "unknown")]
        difficulty: String,

        #[arg(long, default_value = "paired_observed")]
        condition: String,

        #[arg(long = "token-source", default_value = "transcript_context_tokens")]
        token_accounting_source: String,
    },

    /// Reject a pilot task while preserving its local audit trail.
    Reject {
        manifest: PathBuf,

        #[arg(long = "task-id")]
        task_id: String,

        #[arg(long)]
        reason: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProofSprintCommand {
    /// Create a Claude Code observed proof-sprint manifest.
    Init {
        manifest: PathBuf,

        #[arg(long, value_enum, default_value_t = AgentClient::Claude)]
        client: AgentClient,

        #[arg(long, default_value_t = 10)]
        sessions: usize,

        #[arg(long, default_value = "claude-opus-4-8")]
        model: String,

        #[arg(long)]
        force: bool,

        #[arg(long = "skip-repo-check", hide = true)]
        skip_repo_check: bool,
    },

    /// Show observed proof-sprint progress and the next collection command.
    Status {
        manifest: PathBuf,

        /// Accepted for CLI stability. CallSieve output is JSON by default.
        #[arg(long)]
        json: bool,
    },

    /// Collect one baseline or CallSieve Claude Code proof-sprint phase.
    Collect {
        manifest: PathBuf,

        #[arg(long = "task-id", alias = "task_id")]
        task_id: String,

        #[arg(long, value_enum)]
        mode: PilotSessionMode,

        #[arg(long = "max-budget-usd", default_value = "0.50")]
        max_budget_usd: String,

        #[arg(long = "dry-run")]
        dry_run: bool,
    },

    /// Collect the next missing Claude Code proof-sprint phase, resuming safely.
    Run {
        manifest: PathBuf,

        #[arg(long = "max-budget-usd", default_value = "0.50")]
        max_budget_usd: String,

        /// Continue a partially collected manifest by skipping completed phases.
        #[arg(long)]
        resume: bool,

        /// Maximum phases to collect in this invocation.
        #[arg(long, default_value_t = 1)]
        limit: usize,

        #[arg(long = "dry-run")]
        dry_run: bool,
    },

    /// Finalize a proof-sprint manifest into a proof-report artifact after QA passes.
    Finalize {
        manifest: PathBuf,

        #[arg(long)]
        out: PathBuf,

        #[arg(long, default_value_t = REHEARSAL_REPORT_LIMIT)]
        limit: usize,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AgentClient {
    Codex,
    Claude,
    Copilot,
    #[value(name = "opencode")]
    OpenCode,
    Antigravity,
    Cursor,
    #[value(name = "vscode")]
    Vscode,
    Windsurf,
    Continue,
    Zed,
    Junie,
    #[value(name = "jetbrains")]
    JetBrains,
    Amp,
    Goose,
    Warp,
    Cline,
    Zoo,
    Roo,
    Generic,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum McpConfigFormat {
    Json,
    Toml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EmbedModelArg {
    /// BGE-small-en-v1.5: the fast default.
    Small,
    /// jina-embeddings-v2-base-code: code-tuned, heavier, opt-in quality tier.
    Code,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ContextProfileArg {
    Skim,
    Normal,
    Full,
}

impl From<ContextProfileArg> for query::ContextProfile {
    fn from(value: ContextProfileArg) -> Self {
        match value {
            ContextProfileArg::Skim => Self::Skim,
            ContextProfileArg::Normal => Self::Normal,
            ContextProfileArg::Full => Self::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AgentOutputFormat {
    Json,
    Markdown,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SessionPhase {
    Baseline,
    Callsieve,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PilotSessionMode {
    Baseline,
    Callsieve,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EditorKind {
    Vscode,
    Cursor,
    Generic,
}

#[derive(Debug, Serialize, Clone)]
struct IndexOutput {
    command: &'static str,
    root: String,
    index: String,
    files: usize,
    symbols: usize,
    imports: usize,
    references: usize,
    lsp_enriched: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AgentContextOutput {
    instruction: AgentContextInstruction,
    memory: query::TaskMemoryOutput,
    context: query::ContextOutput,
}

#[derive(Debug, Serialize)]
struct AgentContextInstruction {
    #[serde(rename = "x")]
    local_first_expansion: AgentIndexedLocalFirstExpansion,
}

#[derive(Debug, Serialize)]
struct AgentIndexedLocalFirstExpansion {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "o")]
    inspect_top_file: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "n")]
    inspect_next_file: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(rename = "next")]
    inspect_next_files: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "r")]
    expand_relationships: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "t")]
    inspect_tests: Option<u8>,
}

#[derive(Debug, Serialize)]
struct AgentLocalFirstExpansion {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "o")]
    inspect_top_file: Option<AgentFocusExpansion>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(rename = "next")]
    inspect_next_files: Vec<AgentFocusExpansion>,
    #[serde(skip_serializing_if = "skip_false")]
    #[serde(rename = "rel")]
    expand_relationships: bool,
    #[serde(skip_serializing_if = "skip_false")]
    #[serde(rename = "tests")]
    inspect_tests: bool,
}

#[derive(Debug)]
struct AgentFocusExpansion {
    file: String,
    symbol: Option<String>,
    line: Option<usize>,
}

impl Serialize for AgentFocusExpansion {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq;

        let len = if self.line.is_some() {
            3
        } else if self.symbol.is_some() {
            2
        } else {
            1
        };
        let mut seq = serializer.serialize_seq(Some(len))?;
        seq.serialize_element(&self.file)?;
        if self.symbol.is_some() || self.line.is_some() {
            seq.serialize_element(&self.symbol)?;
        }
        if let Some(line) = self.line {
            seq.serialize_element(&line)?;
        }
        seq.end()
    }
}

#[derive(Debug, Serialize)]
struct MemoryClearOutput {
    command: &'static str,
    root: String,
    path: String,
    removed: bool,
}

#[derive(Debug, Serialize)]
struct McpConfigOutput {
    command: &'static str,
    root: String,
    format: &'static str,
    instructions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_text: Option<String>,
}

#[derive(Debug, Serialize)]
struct McpContractOutput {
    command: &'static str,
    #[serde(flatten)]
    contract: PublicMcpContractEvidence,
}

#[derive(Debug, Serialize)]
struct AgentNativeProtocolOutput {
    command: &'static str,
    schema_version: u16,
    status: &'static str,
    claim_boundary: &'static str,
    workflow: Vec<AgentNativeProtocolStep>,
    required_task_fields: AgentNativeProtocolTaskFields,
    source_artifact_requirements: AgentNativeProtocolSourceArtifacts,
    validation_commands: Vec<&'static str>,
    public_proof_integration: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct AgentNativeProtocolStep {
    step: u8,
    name: &'static str,
    command: &'static str,
    evidence: &'static str,
}

#[derive(Debug, Serialize)]
struct AgentNativeProtocolTaskFields {
    template: Vec<&'static str>,
    measured: Vec<&'static str>,
    recording_status: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct AgentNativeProtocolSourceArtifacts {
    required_roles: Vec<&'static str>,
    hash_algorithm: &'static str,
    requirements: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct DemoOutput {
    command: &'static str,
    root: String,
    task: String,
    index: DemoIndexSummary,
    read_first: Vec<String>,
    context_payload_reduction: serde_json::Value,
    next_commands: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DemoIndexSummary {
    path: String,
    files: usize,
    symbols: usize,
    imports: usize,
    references: usize,
    lsp_enriched: bool,
}

#[derive(Debug, Serialize)]
struct WatchOutput {
    command: &'static str,
    root: String,
    mode: String,
    refreshed: bool,
    status: query::IndexStatusOutput,
}

#[derive(Debug, Serialize)]
struct SetupAgentOutput {
    command: &'static str,
    client: String,
    root: String,
    files: Vec<String>,
    first_required_command: String,
    policy: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AutomationStep {
    step: String,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct BootstrapOutput {
    command: &'static str,
    status: String,
    root: String,
    client: String,
    strict: bool,
    steps: Vec<AutomationStep>,
    generated_files: Vec<String>,
    daemon: DaemonState,
    first_required_command: String,
    enforcement: EnforceOutput,
}

#[derive(Debug, Serialize)]
struct DoctorOutput {
    command: &'static str,
    status: String,
    root: String,
    client: String,
    message: String,
    checks: Vec<EnforceCheck>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fixes: Vec<AutomationStep>,
}

#[derive(Debug, Serialize)]
struct GuardOutput {
    command: &'static str,
    root: String,
    task: String,
    policy: &'static str,
    local_first_expansion: AgentLocalFirstExpansion,
    context: query::ContextOutput,
    trace_event: AuditEvent,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct BeginOutput {
    command: &'static str,
    root: String,
    client: String,
    task: String,
    policy: &'static str,
    local_first_expansion: AgentLocalFirstExpansion,
    next_step: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    proof_next_commands: Vec<String>,
    context: query::ContextOutput,
    trace_event: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct CodexSessionOutput {
    command: &'static str,
    root: String,
    client: &'static str,
    model: String,
    task: String,
    policy: &'static str,
    local_first_expansion: AgentLocalFirstExpansion,
    context: query::ContextOutput,
    trace: query::TraceReplayOutput,
    trace_event: AuditEvent,
    trace_path: String,
}

#[derive(Debug, Serialize)]
struct GrepOutput {
    command: &'static str,
    policy: &'static str,
    rg_status: &'static str,
    context: serde_json::Value,
    audit_event: AuditEvent,
    #[serde(skip_serializing_if = "Option::is_none")]
    rg: Option<RgOutput>,
}

#[derive(Debug, Serialize)]
struct AuditEvent {
    tool: &'static str,
    policy: &'static str,
    context_first: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    rg_ran: Option<bool>,
    called_at: u64,
}

#[derive(Debug, Serialize)]
struct RgOutput {
    status_code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Serialize)]
struct EvidencePackOutput {
    command: &'static str,
    anonymized: bool,
    generated_at: u64,
    protocol: String,
    evidence: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CompetitiveReportOutput {
    command: &'static str,
    status: String,
    generated_at: u64,
    goal: String,
    targets: CompetitiveReportTargetsOutput,
    callsieve: CompetitiveCallsieveEvidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    natural_language: Option<CompetitiveCallsieveEvidence>,
    comparison_matrix: Vec<CompetitiveComparisonRow>,
    benchmark: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    natural_language_benchmark: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    performance: Option<serde_json::Value>,
    trace_policy: serde_json::Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failures: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    next_actions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CompetitiveReportTargetsOutput {
    minimum_first_correct_file_rate_at_k: f64,
    minimum_expected_file_recall: f64,
    minimum_natural_language_first_correct_file_rate_at_k: f64,
    minimum_natural_language_expected_file_recall: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    maximum_first_query_p95_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    maximum_default_packet_tokens: Option<usize>,
    require_natural_language_benchmark: bool,
    require_trace_or_receipt_proof: bool,
    require_zero_retrieval_model_tokens: bool,
    require_zero_per_query_retrieval_cost: bool,
    require_no_default_code_upload: bool,
    minimum_agent_coverage: usize,
    required_agents: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CompetitiveCallsieveEvidence {
    evidence_tier: &'static str,
    locally_measured: bool,
    first_correct_file_rate_at_k: f64,
    expected_file_recall: f64,
    default_packet_tokens: usize,
    baseline_context_payload_tokens: usize,
    callsieve_context_payload_tokens: usize,
    context_payload_reduction_percent: f64,
    packet_quality: PublicContextPacketQualityEvidence,
    avoided_grep_commands: usize,
    avoided_file_reads: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_query_p50_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_query_p95_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_query_samples: Option<usize>,
    retrieval_model_tokens: usize,
    per_query_retrieval_cost_usd: f64,
    default_code_upload_required: bool,
    explainability: &'static str,
    agent_coverage: Vec<&'static str>,
    agent_coverage_count: usize,
    missing_required_agents: Vec<String>,
    strict_trace_policy_sessions: usize,
    strict_trace_policy_violations: usize,
    context_first_compliant: bool,
    trace_or_receipt_proof_available: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
struct PublicContextPacketQualityEvidence {
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

#[derive(Debug, Serialize)]
struct CompetitiveComparisonRow {
    name: String,
    category: String,
    evidence_kind: &'static str,
    source: String,
    install_friction: String,
    first_query_time: String,
    code_upload_requirement: String,
    retrieval_model_token_cost: String,
    per_query_retrieval_cost: String,
    packet_compactness: String,
    first_correct_file_rate_at_k: String,
    natural_language_recall: String,
    explainability: String,
    agent_coverage: String,
    trace_or_receipt_proof: String,
    where_callsieve_should_win: String,
}

#[derive(Debug, Serialize)]
struct PublicProofReportOutput {
    command: &'static str,
    status: String,
    generated_at: u64,
    goal: String,
    local_gate: PublicProofLocalGate,
    targets: PublicProofTargetsOutput,
    public_benchmarks: Vec<PublicBenchmarkEvidence>,
    public_result_catalog: PublicResultCatalog,
    public_checkouts: PublicCheckoutEvidence,
    broad_read_guardrail: PublicBroadReadGuardrail,
    context_packet_guardrail: PublicContextPacketGuardrail,
    evidence_pack: PublicEvidencePack,
    #[serde(skip_serializing_if = "Option::is_none")]
    miss_triage: Option<PublicMissTriage>,
    repo_packer_guardrail: PublicRepoPackerGuardrail,
    agent_native_search_guardrail: PublicAgentNativeSearchGuardrail,
    mcp_surface: PublicMcpSurfaceEvidence,
    mcp_contract: PublicMcpContractEvidence,
    agent_setup: PublicAgentSetupEvidence,
    evidence_commands: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    failures: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    next_actions: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PublicProofTargetsOutput {
    require_competitive_report_pass: bool,
    require_zero_retrieval_model_tokens: bool,
    require_zero_per_query_retrieval_cost: bool,
    require_no_default_code_upload: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    maximum_default_packet_tokens: Option<usize>,
    require_broad_read_payload_reduction: bool,
    require_context_packet_quality: bool,
    require_current_public_report_retrieval_contract: bool,
    minimum_broad_read_context_payload_reduction_percent: f64,
    minimum_public_reports: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_total_public_evaluated: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_agent_native_measured_baselines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_agent_native_distinct_tools: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_agent_native_repositories: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_agent_native_task_languages: Option<usize>,
    target_swe_bench_style_first_correct_file_rate_at_k: f64,
    target_natural_language_first_correct_file_rate_at_k: f64,
}

#[derive(Debug, Serialize)]
struct PublicProofLocalGate {
    manifest: String,
    status: String,
    expected_file_recall: f64,
    first_correct_file_rate_at_k: f64,
    natural_language_first_correct_file_rate_at_k: Option<f64>,
    retrieval_model_tokens: usize,
    per_query_retrieval_cost_usd: f64,
    default_code_upload_required: bool,
    default_packet_tokens: usize,
    first_query_p95_ms: Option<u64>,
    trace_or_receipt_proof_available: bool,
    agent_coverage_count: usize,
}

#[derive(Debug, Serialize)]
struct PublicBenchmarkEvidence {
    id: String,
    category: String,
    report: String,
    manifest: String,
    retrieval_contract_fingerprint: Option<String>,
    retrieval_contract_status: String,
    requires_current_retrieval_contract: bool,
    k: usize,
    evaluated: usize,
    skipped: usize,
    total: usize,
    preferred_arm: String,
    first_correct_file_rate_at_k: f64,
    grep_first_correct_file_rate_at_k: Option<f64>,
    minus_grep: Option<f64>,
    average_selected_files: f64,
    average_grep_matched_files: Option<f64>,
    misses: Vec<String>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicResultCatalog {
    reports: Vec<PublicResultCatalogEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_swe_bench_style: Option<PublicResultCatalogBest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    best_natural_language: Option<PublicResultCatalogBest>,
}

#[derive(Debug, Clone, Serialize)]
struct PublicResultCatalogEntry {
    report: String,
    manifest: String,
    category: String,
    retrieval_contract_fingerprint: Option<String>,
    retrieval_contract_status: String,
    k: usize,
    evaluated: usize,
    lexical_first_correct_file_rate_at_k: f64,
    hybrid_first_correct_file_rate_at_k: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    grep_first_correct_file_rate_at_k: Option<f64>,
    best_arm: String,
    best_first_correct_file_rate_at_k: f64,
}

#[derive(Debug, Clone, Serialize)]
struct PublicResultCatalogBest {
    report: String,
    category: String,
    best_arm: String,
    first_correct_file_rate_at_k: f64,
    target_first_correct_file_rate_at_k: f64,
    gap_to_target: f64,
    status: String,
}

#[derive(Debug, Serialize)]
struct PublicRepoPackerGuardrail {
    status: String,
    claim_scope: String,
    default_packet_tokens: usize,
    maximum_default_packet_tokens: Option<usize>,
    average_public_selected_files: f64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    baselines: Vec<PublicRepoPackerBaselineEvidence>,
    evidence: String,
    next_baseline: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicBroadReadGuardrail {
    status: String,
    claim_scope: String,
    baseline_context_payload_tokens: usize,
    callsieve_context_payload_tokens: usize,
    context_payload_tokens_saved: isize,
    context_payload_reduction_percent: f64,
    minimum_context_payload_reduction_percent: f64,
    avoided_file_reads: usize,
    avoided_grep_commands: usize,
    evidence: String,
    next_baseline: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicContextPacketGuardrail {
    status: String,
    claim_scope: String,
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
    evidence: String,
    next_baseline: String,
}

#[derive(Debug, Serialize)]
struct PublicAgentNativeSearchGuardrail {
    status: String,
    claim_scope: String,
    summary: PublicAgentNativeSearchSummary,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    baselines: Vec<PublicAgentNativeSearchBaselineEvidence>,
    evidence: String,
    next_baseline: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicAgentNativeSearchSummary {
    baseline_count: usize,
    required_baseline_count: usize,
    measured_baseline_count: usize,
    transcript_backed_baseline_count: usize,
    total_measured_tasks: usize,
    distinct_agent_tool_count: usize,
    agent_tools: Vec<String>,
    distinct_repository_count: usize,
    repositories: Vec<String>,
    distinct_base_commit_count: usize,
    base_commits: Vec<String>,
    task_language_count: usize,
    task_languages: Vec<String>,
    single_agent_only: bool,
    multi_agent_status: String,
    evidence: String,
    next_baseline: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicMcpSurfaceEvidence {
    status: String,
    protocol: &'static str,
    required_first_mile_tools: Vec<String>,
    exposed_tools: Vec<String>,
    missing_required_tools: Vec<String>,
    context_first_tool: &'static str,
    local_config_commands: Vec<&'static str>,
    registry_manifest_command: &'static str,
    evidence: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicMcpContractEvidence {
    status: String,
    contract_version: u16,
    context_tool: &'static str,
    structured_content_required: bool,
    default_profile: &'static str,
    default_token_budget: usize,
    required_structured_content_fields: Vec<&'static str>,
    supported_instruction_expansion_keys: Vec<&'static str>,
    required_freshness_fields: Vec<&'static str>,
    evidence: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct PublicCheckoutEvidence {
    status: String,
    repos_dir: String,
    manifests: Vec<String>,
    expected_repos: usize,
    present_repos: usize,
    missing_repos: Vec<String>,
    missing_manifests: Vec<String>,
    commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct PublicRepoPackerBaselineEvidence {
    id: String,
    tool: String,
    path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    command: String,
    required: bool,
    exists: bool,
    locally_measured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<usize>,
    #[serde(skip_serializing_if = "skip_false")]
    tokens_estimated_from_bytes: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_ratio_vs_callsieve_packet: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_token_ratio_vs_callsieve_packet: Option<f64>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicAgentNativeSearchBaselineEvidence {
    id: String,
    tool: String,
    path: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    command: String,
    required: bool,
    exists: bool,
    locally_measured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tasks: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    repositories: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    base_commits: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    task_languages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_native_first_correct_file_rate_at_k: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    callsieve_first_correct_file_rate_at_k: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    callsieve_minus_agent_native_first_correct_file_rate_at_k: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_native_average_context_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    callsieve_average_packet_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_native_context_token_ratio_vs_callsieve: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_artifacts: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    source_artifact_hashes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    transcript_source_artifact_hashes: Vec<String>,
    transcript_provenance_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_tasks: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    minimum_agent_native_context_token_ratio_vs_callsieve: Option<f64>,
    status: String,
}

#[derive(Debug, Clone, Serialize)]
struct PublicAgentSetupEvidence {
    status: String,
    claim_scope: &'static str,
    required_agents: Vec<String>,
    covered_agents: Vec<String>,
    missing_required_agents: Vec<String>,
    agent_coverage_count: usize,
    minimum_agent_coverage: usize,
    default_layer_clients: Vec<String>,
    default_layer_clients_covered: Vec<String>,
    missing_default_layer_clients: Vec<String>,
    priority_clients: Vec<&'static str>,
    priority_clients_covered: Vec<&'static str>,
    hook_capable_clients: Vec<&'static str>,
    mcp_template_clients: Vec<&'static str>,
    commands: Vec<String>,
    evidence: &'static str,
    next_step: &'static str,
}

#[derive(Debug, Serialize)]
struct PublicEvidencePack {
    commands: Vec<String>,
    fixture_manifests: Vec<String>,
    result_reports: Vec<String>,
    metrics: Vec<PublicProofMetric>,
    misses: Vec<PublicProofMissEvidence>,
    receipt_commands: Vec<String>,
    terminal_artifacts: Vec<PublicProofArtifactEvidence>,
    public_checkouts: PublicCheckoutEvidence,
    broad_read_guardrail: PublicBroadReadGuardrail,
    context_packet_guardrail: PublicContextPacketGuardrail,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    repo_packer_baselines: Vec<PublicRepoPackerBaselineEvidence>,
    agent_native_search_summary: PublicAgentNativeSearchSummary,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    agent_native_search_baselines: Vec<PublicAgentNativeSearchBaselineEvidence>,
    mcp_surface: PublicMcpSurfaceEvidence,
    mcp_contract: PublicMcpContractEvidence,
    agent_setup: PublicAgentSetupEvidence,
}

#[derive(Debug, Serialize)]
struct PublicProofMetric {
    name: String,
    value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<serde_json::Value>,
    status: String,
}

#[derive(Debug, Serialize)]
struct PublicProofMissEvidence {
    report: String,
    preferred_arm: String,
    sampled_misses: usize,
    sample: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PublicProofArtifactEvidence {
    id: String,
    path: String,
    kind: String,
    description: String,
    required: bool,
    exists: bool,
    status: String,
    validation: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    source_artifact_hashes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PublicMissTriage {
    path: String,
    total: usize,
    reachable: usize,
    not_reachable: usize,
    same_directory_hints: usize,
    sample: Vec<PublicMissTriageSample>,
}

#[derive(Debug, Serialize)]
struct PublicMissTriageSample {
    id: String,
    truth: String,
    reachable: bool,
    same_dir: Vec<String>,
    pool_refs_truth: Vec<String>,
    truth_refs_pool: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PolicyCheckOutput {
    command: &'static str,
    trace: String,
    check: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofRehearsalLedger {
    schema_version: u32,
    command_matrix: String,
    status: String,
    started_at: u64,
    updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    finished_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    fixes: Vec<ProofRehearsalFix>,
    steps: Vec<ProofRehearsalStepRecord>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofRehearsalFix {
    fix: String,
    path: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ProofRehearsalStepRecord {
    id: String,
    description: String,
    command: String,
    signature: String,
    status: String,
    skipped: bool,
    attempts: usize,
    started_at: u64,
    finished_at: u64,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    summary: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProofRehearsalOutput {
    command: &'static str,
    status: String,
    mode: &'static str,
    ledger: String,
    command_matrix: ProofRehearsalCommandMatrix,
    preflight: ProofRehearsalPreflightOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_payload_reduction: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fixes: Vec<ProofRehearsalFix>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    steps: Vec<ProofRehearsalStepRecord>,
    claim_proof_included: bool,
    next_observed_gate: String,
}

#[derive(Debug, Serialize)]
struct ProofRehearsalCommandMatrix {
    retrieval_fixtures: String,
    external_manifest: String,
    external_fixtures: Vec<ExternalBenchmarkFixture>,
    report_limit: usize,
    perf_iterations: usize,
    context_payload_reduction_included: bool,
    context_payload_scope: &'static str,
    includes_proof_report: bool,
}

#[derive(Debug, Serialize)]
struct ProofRehearsalPreflightOutput {
    status: String,
    failures: usize,
    checks: Vec<ProofRehearsalCheck>,
}

#[derive(Debug, Serialize)]
struct ProofRehearsalCheck {
    check: String,
    path: String,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct ObservedSetupOutput {
    command: &'static str,
    status: String,
    manifest: String,
    task_count: usize,
    target_sessions: usize,
    repos: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bootstrap: Vec<serde_json::Value>,
    next_qa: String,
    final_proof: String,
}

#[derive(Debug, Serialize)]
struct RecordObservedSessionOutput {
    command: &'static str,
    status: String,
    manifest: String,
    client: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    task_id: String,
    mode: PilotSessionMode,
    files_read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context_selected_files: Vec<String>,
    tokens: usize,
    token_accounting_source: &'static str,
    token_input_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage_breakdown: Option<ClaudeCodeUsageBreakdown>,
    pilot_run_command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pilot_run: Option<serde_json::Value>,
    next_qa: String,
}

#[derive(Debug, Serialize)]
struct CollectClaudeObservedSessionOutput {
    command: &'static str,
    status: String,
    manifest: String,
    task_id: String,
    mode: PilotSessionMode,
    repo: String,
    model: String,
    artifact: String,
    max_budget_usd: String,
    context_limit: usize,
    snippets_per_file: usize,
    allowed_tools: Vec<String>,
    prompt_tokens_estimate: usize,
    context_selected_files: Vec<String>,
    claude_command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    record: Option<RecordObservedSessionOutput>,
}

#[derive(Debug, Serialize)]
struct ProofSprintInitOutput {
    command: &'static str,
    status: String,
    manifest: String,
    client: String,
    model: String,
    task_count: usize,
    target_sessions: usize,
    repos: Vec<String>,
    next_status: String,
    next_collect: String,
    final_proof: String,
}

#[derive(Debug, Serialize)]
struct ProofSprintStatusOutput {
    command: &'static str,
    manifest: String,
    status: String,
    client: String,
    target_sessions: usize,
    planned_tasks: usize,
    paired_sessions_complete: usize,
    rejected_sessions: usize,
    missing_baseline_phases: Vec<String>,
    missing_callsieve_phases: Vec<String>,
    observed_token_reduction_percent: Option<f64>,
    critical_misses: usize,
    strict_trace_violations: usize,
    transcript_accounting_coverage_percent: f64,
    qa_status: String,
    qa_failures: usize,
    next_command: String,
}

#[derive(Debug, Serialize)]
struct ProofSprintCollectOutput {
    command: &'static str,
    status: String,
    manifest: String,
    task_id: String,
    mode: PilotSessionMode,
    model: String,
    collect: CollectClaudeObservedSessionOutput,
    next_status: String,
}

#[derive(Debug, Serialize)]
struct ProofSprintRunOutput {
    command: &'static str,
    status: String,
    manifest: String,
    resume: bool,
    dry_run: bool,
    requested_limit: usize,
    collected_phases: usize,
    phases: Vec<ProofSprintRunPhaseOutput>,
    status_after: ProofSprintStatusOutput,
    next_command: String,
}

#[derive(Debug, Serialize)]
struct ProofSprintRunPhaseOutput {
    task_id: String,
    mode: PilotSessionMode,
    status: String,
    collect: ProofSprintCollectOutput,
}

#[derive(Debug, Serialize)]
struct ProofSprintFinalizeOutput {
    command: &'static str,
    status: String,
    manifest: String,
    out: String,
    finalize: PilotFinalizeOutput,
}

#[derive(Debug, Clone, Serialize)]
struct ClaudeCodeUsageBreakdown {
    input_tokens: usize,
    cache_creation_input_tokens: usize,
    cache_read_input_tokens: usize,
    output_tokens: usize,
    total_tokens: usize,
}

#[derive(Debug, Clone)]
struct ObservedTokenInput {
    tokens: usize,
    token_input_source: String,
    usage_json: Option<String>,
    usage_breakdown: Option<ClaudeCodeUsageBreakdown>,
}

#[derive(Debug, Clone)]
struct ObservedCodexTaskRow {
    id: String,
    repo: String,
    suite: String,
    task: String,
    expected_files: Vec<String>,
    critical_files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PerfReportOutput {
    command: &'static str,
    status: String,
    root: String,
    iterations: usize,
    task_count: usize,
    summary: PerfLatencySummary,
    tasks: Vec<PerfTaskOutput>,
}

#[derive(Debug, Serialize)]
struct PerfLatencySummary {
    samples: usize,
    p50_ms: u64,
    p95_ms: u64,
    min_ms: u64,
    max_ms: u64,
}

#[derive(Debug, Serialize)]
struct PerfTaskOutput {
    task: String,
    samples: usize,
    p50_ms: u64,
    p95_ms: u64,
    min_ms: u64,
    max_ms: u64,
}

#[derive(Debug, Serialize)]
struct SessionStartOutput {
    command: &'static str,
    trace: String,
    task: String,
    client: String,
    model: String,
    collection: &'static str,
    first_required_command: String,
}

#[derive(Debug, Serialize)]
struct SessionEventOutput {
    command: &'static str,
    trace: String,
    event: serde_json::Value,
    summary: query::TraceSummaryOutput,
}

#[derive(Debug, Serialize)]
struct SessionFinishOutput {
    command: &'static str,
    trace: String,
    out: String,
    summary: query::TraceSummaryOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_correct_file_rate_at_k: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_correct_file_rate_k: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turns_to_first_edit: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wrong_files_read: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PilotHarnessManifest {
    schema_version: u32,
    target_sessions: usize,
    #[serde(default = "default_pilot_protocol")]
    protocol: PilotEvidenceProtocol,
    thresholds: serde_json::Value,
    #[serde(default)]
    tasks: Vec<PilotHarnessTask>,
    #[serde(default)]
    rejected_sessions: Vec<PilotRejectedSession>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PilotEvidenceProtocol {
    evidence_standard: String,
    collection: String,
    pairing: String,
    token_accounting: String,
    controlled_replay_policy: String,
    planned_task_buffer_ratio: f64,
    minimum_planned_tasks: usize,
    qa_batch_size: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PilotHarnessTask {
    id: String,
    repo: String,
    task: String,
    client: String,
    model: String,
    #[serde(default)]
    expected_files: Vec<String>,
    #[serde(default)]
    critical_files: Vec<String>,
    #[serde(default)]
    external: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    suite_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pair_id: Option<String>,
    #[serde(default = "default_pilot_task_category")]
    task_category: String,
    #[serde(default = "default_pilot_difficulty")]
    difficulty: String,
    #[serde(default = "default_pilot_condition")]
    condition: String,
    #[serde(default = "default_pilot_token_accounting_source")]
    token_accounting_source: String,
    #[serde(default = "default_true")]
    preregistered: bool,
    baseline_trace_path: String,
    callsieve_trace_path: String,
    trace_path: String,
    summary_path: String,
    status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct PilotRejectedSession {
    task_id: String,
    reason: String,
    status_at_rejection: String,
    trace_path: String,
    rejected_at: u64,
}

#[derive(Debug, Serialize)]
struct PilotInitOutput {
    command: &'static str,
    manifest: String,
    target_sessions: usize,
    status: String,
}

#[derive(Debug, Serialize)]
struct PilotTaskAddOutput {
    command: &'static str,
    manifest: String,
    task: PilotHarnessTask,
}

#[derive(Debug, Serialize)]
struct PilotTaskRejectOutput {
    command: &'static str,
    manifest: String,
    rejected: PilotRejectedSession,
}

#[derive(Debug, Serialize)]
struct PilotRunOutput {
    command: &'static str,
    manifest: String,
    task_id: String,
    mode: PilotSessionMode,
    trace: String,
    mode_trace: String,
    summary: query::TraceSummaryOutput,
}

#[derive(Debug, Serialize)]
struct PilotCollectLocalOutput {
    command: &'static str,
    manifest: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    requested_sessions: usize,
    collected_sessions: usize,
    skipped_sessions: usize,
    observed_sessions: usize,
    qa_status: String,
    sessions: Vec<PilotCollectLocalSessionOutput>,
}

#[derive(Debug, Serialize)]
struct PilotCollectLocalSessionOutput {
    task_id: String,
    repo: String,
    status: String,
    baseline_tokens: usize,
    callsieve_tokens: usize,
    token_reduction_percent: f64,
    baseline_files: usize,
    callsieve_files: usize,
    baseline_artifact: String,
    callsieve_artifact: String,
}

#[derive(Debug, Serialize)]
struct OllamaTranscriptArtifact {
    schema_version: u32,
    collection: &'static str,
    collector: &'static str,
    task_id: String,
    phase: String,
    repo: String,
    model: String,
    command: String,
    files_read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context_selected_files: Vec<String>,
    prompt: String,
    response: String,
    token_accounting: OllamaTokenAccounting,
    created_at: u64,
}

#[derive(Debug, Serialize)]
struct OllamaTokenAccounting {
    source: &'static str,
    counted_tokens: usize,
    prompt_eval_count: usize,
    eval_count: usize,
}

struct OllamaRun {
    response: String,
    prompt_eval_count: usize,
    eval_count: usize,
}

#[derive(Debug, Serialize)]
struct LmStudioTranscriptArtifact {
    schema_version: u32,
    collection: &'static str,
    collector: &'static str,
    task_id: String,
    phase: String,
    repo: String,
    model: String,
    base_url: String,
    command: String,
    files_read: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    context_selected_files: Vec<String>,
    prompt: String,
    response: String,
    token_accounting: LmStudioTokenAccounting,
    raw_response: serde_json::Value,
    created_at: u64,
}

#[derive(Debug, Serialize)]
struct LmStudioTokenAccounting {
    source: &'static str,
    counted_tokens: usize,
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
}

struct LmStudioRun {
    response: String,
    prompt_tokens: usize,
    completion_tokens: usize,
    total_tokens: usize,
    raw_response: serde_json::Value,
}

struct HttpEndpoint {
    host: String,
    port: u16,
    path: String,
}

struct PilotPromptPlan {
    command: String,
    files_read: Vec<String>,
    context_selected_files: Vec<String>,
    prompt: String,
}

struct FileSearchEvidence {
    path: String,
    content: String,
    match_lines: Vec<usize>,
    result_lines: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PilotQaOutput {
    command: &'static str,
    manifest: String,
    status: String,
    target_sessions: usize,
    observed_sessions: usize,
    rejected_sessions: usize,
    tasks: usize,
    failures: usize,
    results: Vec<PilotQaCheck>,
}

#[derive(Debug, Serialize)]
struct PilotQaCheck {
    task_id: String,
    check: String,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct PilotFinalizeOutput {
    command: &'static str,
    manifest: String,
    proof_manifest: String,
    out: String,
    qa: PilotQaOutput,
    proof: query::ProofReportOutput,
}

fn default_pilot_protocol() -> PilotEvidenceProtocol {
    PilotEvidenceProtocol {
        evidence_standard: "observed_session_only".to_string(),
        collection: "real_paired_developer_sessions".to_string(),
        pairing: "paired_baseline_and_callsieve_phases".to_string(),
        token_accounting: "transcript_context_tokens".to_string(),
        controlled_replay_policy: "reported_separately_never_counted_as_observed".to_string(),
        planned_task_buffer_ratio: 1.2,
        minimum_planned_tasks: 0,
        qa_batch_size: 10,
    }
}

fn default_pilot_task_category() -> String {
    "code_change".to_string()
}

fn default_pilot_difficulty() -> String {
    "unknown".to_string()
}

fn default_pilot_condition() -> String {
    "paired_observed".to_string()
}

fn default_pilot_token_accounting_source() -> String {
    "transcript_context_tokens".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
struct EnforceOutput {
    command: &'static str,
    status: String,
    root: String,
    client: String,
    checks: Vec<EnforceCheck>,
}

#[derive(Debug, Serialize, Clone)]
struct EnforceCheck {
    check: String,
    status: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct ShimOutput {
    command: &'static str,
    status: String,
    root: String,
    bin_dir: String,
    strict: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace: Option<String>,
    files: Vec<String>,
    path_instruction: String,
}

#[derive(Debug, Serialize)]
struct ShimDoctorOutput {
    command: &'static str,
    status: String,
    root: String,
    checks: Vec<EnforceCheck>,
    path_instruction: String,
}

#[derive(Debug, Serialize)]
struct HookInstallOutput {
    command: &'static str,
    status: String,
    root: String,
    client: String,
    strict: bool,
    index: IndexOutput,
    setup: SetupAgentOutput,
    shim: ShimOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    codex_hooks: Option<CodexHooksInstallOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_hooks: Option<ClaudeHooksInstallOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_hooks: Option<ClientHooksInstallOutput>,
    launchers: Vec<String>,
    first_required_command: String,
    path_instruction: String,
    policy: &'static str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HookDoctorOutput {
    command: &'static str,
    status: String,
    root: String,
    checks: Vec<EnforceCheck>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    integrations: Vec<HookDoctorIntegration>,
}

#[derive(Debug, Serialize)]
struct HookDoctorIntegration {
    client: String,
    status: String,
    profile: String,
    hooks_file: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    events: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CodexHooksInstallOutput {
    command: &'static str,
    status: String,
    root: String,
    profile: &'static str,
    strict: bool,
    hooks_file: String,
    trace_dir: String,
    files: Vec<String>,
    index: IndexOutput,
    first_required_command: String,
    trust_instruction: &'static str,
    policy: &'static str,
}

#[derive(Debug, Serialize)]
struct CodexHooksDoctorOutput {
    command: &'static str,
    status: String,
    root: String,
    profile: &'static str,
    strict: bool,
    hooks_file: String,
    trace_dir: String,
    checks: Vec<EnforceCheck>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fixes: Vec<String>,
    trust: CodexHookTrustReview,
    trust_instruction: &'static str,
}

#[derive(Debug, Clone, Copy, Default)]
struct CodexHooksDoctorOptions {
    smoke: bool,
    fix: bool,
}

#[derive(Debug, Serialize)]
struct CodexHookTrustReview {
    status: String,
    trust_file: String,
    hooks_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reviewed_at: Option<u64>,
    message: String,
}

#[derive(Debug, Serialize)]
struct CodexHooksTrustAckOutput {
    command: &'static str,
    status: String,
    root: String,
    profile: &'static str,
    hooks_file: String,
    trust_file: String,
    hooks_hash: String,
    reviewed_at: u64,
    manual_review: &'static str,
}

#[derive(Debug, Serialize)]
struct CodexHooksUninstallOutput {
    command: &'static str,
    status: String,
    root: String,
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ClaudeHooksInstallOutput {
    command: &'static str,
    status: String,
    root: String,
    strict: bool,
    hooks_file: String,
    trace_dir: String,
    files: Vec<String>,
    index: IndexOutput,
    first_required_command: String,
    trust_instruction: &'static str,
    policy: &'static str,
}

#[derive(Debug, Serialize)]
struct ClaudeHooksDoctorOutput {
    command: &'static str,
    status: String,
    root: String,
    strict: bool,
    hooks_file: String,
    trace_dir: String,
    checks: Vec<EnforceCheck>,
    trust_instruction: &'static str,
}

#[derive(Debug, Serialize)]
struct ClaudeHooksUninstallOutput {
    command: &'static str,
    status: String,
    root: String,
    files: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ClientHooksInstallOutput {
    command: String,
    status: String,
    root: String,
    client: String,
    strict: bool,
    hooks_file: String,
    trace_dir: String,
    files: Vec<String>,
    index: IndexOutput,
    first_required_command: String,
    trust_instruction: String,
    policy: String,
}

#[derive(Debug, Serialize)]
struct ClientHooksDoctorOutput {
    command: String,
    status: String,
    root: String,
    client: String,
    strict: bool,
    hooks_file: String,
    trace_dir: String,
    checks: Vec<EnforceCheck>,
    trust_instruction: String,
}

#[derive(Debug, Serialize)]
struct ClientHooksUninstallOutput {
    command: String,
    status: String,
    root: String,
    client: String,
    files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookClient {
    Copilot,
    OpenCode,
    Antigravity,
    Cline,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct CodexHookState {
    version: u32,
    session_id: String,
    turn_id: String,
    root: String,
    strict: bool,
    context_seen: bool,
    violation_seen: bool,
    stop_blocked: bool,
    last_prompt_hash: String,
    #[serde(default)]
    last_prompt: String,
    selected_files: Vec<String>,
    updated_at: u64,
    #[serde(default)]
    context_packets: usize,
    #[serde(default)]
    context_tokens: usize,
    #[serde(default)]
    context_files: usize,
    /// Files the agent actually read after receiving context — the local
    /// training signal for task memory. Capped, deduplicated.
    #[serde(default)]
    confirmed_reads: Vec<String>,
    /// The retrieval task the current packet was built from (anaphoric
    /// follow-ups are recovered into a full task); confirmed reads are keyed
    /// to this, never to a raw "fix it"-style prompt.
    #[serde(default)]
    last_retrieval_task: String,
}

#[derive(Debug, Serialize)]
struct ShimRunOutput {
    command: &'static str,
    root: String,
    tool: &'static str,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shim_event: Option<serde_json::Value>,
    passthrough: RgOutput,
}

#[derive(Debug, Serialize)]
struct CodexBootstrapOutput {
    command: &'static str,
    root: String,
    model: String,
    files: Vec<String>,
    first_required_command: String,
    launcher: Vec<String>,
    policy: &'static str,
}

#[derive(Debug, Serialize)]
struct EditorHookOutput {
    command: &'static str,
    root: String,
    editor: String,
    files: Vec<String>,
    daemon_command: String,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose)?;
    output::json::set_pretty(cli.pretty);
    let tokenizer = query::tokens::TokenizerKind::parse(&cli.tokenizer).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown --tokenizer '{}' (expected heuristic, o200k, or cl100k)",
            cli.tokenizer
        )
    })?;
    query::tokens::set_active(tokenizer);
    query::ranker::set_bm25(cli.bm25);

    match cli.command {
        Command::Index {
            path,
            lsp,
            embeddings,
            embed_model,
            ..
        } => {
            if embeddings {
                ensure_embeddings_supported()?;
            }
            let index = if lsp {
                indexer::build_index_with_options(
                    &path,
                    indexer::IndexOptions {
                        lsp,
                        ..indexer::IndexOptions::default()
                    },
                )?
            } else {
                indexer::build_index(&path)?
            };
            let index_path = store::json_store::save_index(&path, &index)?;
            if embeddings {
                build_embeddings_cache(&path, &index, embed_model)?;
            } else {
                remove_embeddings_cache_if_present(&path)?;
            }
            let output = IndexOutput {
                command: "index",
                root: root_label(&path),
                index: repo_relative_display(&path, &index_path),
                files: index.files.len(),
                symbols: index.symbols.len(),
                imports: index.imports.len(),
                references: index.references.len(),
                lsp_enriched: index.metadata.lsp_enriched,
                warnings: index.warnings,
            };
            output::json::print(&output)?;
        }
        Command::Symbols { path, limit } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::list_symbols(&path, &index, limit)?;
            output::json::print(&output)?;
        }
        Command::Symbol {
            path,
            symbol_name,
            limit,
        } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::find_symbol(&path, &index, &symbol_name, limit)?;
            output::json::print(&output)?;
        }
        Command::Focus {
            path,
            file,
            symbol,
            line,
            references,
            snippets_per_symbol,
            skeleton,
        } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::focus_file(
                &path,
                &index,
                &file,
                symbol.as_deref(),
                line,
                references,
                snippets_per_symbol,
                skeleton,
            )?;
            output::json::print(&output)?;
        }
        Command::Related { path, file } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::related_file(&path, &index, &file)?;
            output::json::print(&output)?;
        }
        Command::Tests { path, file } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::tests_for_file(&path, &index, &file)?;
            output::json::print(&output)?;
        }
        Command::Query {
            path,
            question,
            limit,
            no_snippets,
            why_debug,
        } => {
            let (index, index_load_ms) = load_index_timed(&path)?;
            let mut output = if why_debug {
                query::run_query_with_options(&path, &index, &question, limit, !no_snippets, true)?
            } else {
                query::run_query(&path, &index, &question, limit, !no_snippets)?
            };
            output.add_index_load_time(index_load_ms);
            output::json::print(&output)?;
        }
        Command::Context {
            path,
            task,
            limit,
            snippets_per_file,
            no_snippets,
            why_debug,
            profile,
            token_budget,
            format,
        } => {
            let (index, index_load_ms, refreshed) = load_fresh_index_timed(&path)?;
            let mut output = query::build_context_with_options(
                &path,
                &index,
                &task,
                limit,
                snippets_per_file,
                !no_snippets,
                why_debug,
            )?;
            output.add_index_load_time(index_load_ms);
            if refreshed {
                output.add_warning("rebuilt missing or stale CallSieve index before context");
            }
            print_context_output(
                &output,
                format,
                query::ContextViewOptions {
                    profile: profile.into(),
                    token_budget,
                    include_git: false,
                    include_call_paths: false,
                },
            )?;
        }
        Command::AgentContext {
            path,
            task,
            limit,
            snippets_per_file,
            why_debug,
            profile,
            token_budget,
            format,
            embeddings,
            error,
            git_boost,
            no_daemon,
            embed_model,
            memory_boost,
        } => {
            let embeddings = embeddings || embeddings_env_enabled();
            if embeddings {
                ensure_embeddings_supported()?;
            }
            let request = AgentContextRequest {
                task: task.clone(),
                limit,
                snippets_per_file,
                why_debug,
                profile: profile_arg_name(profile).to_string(),
                token_budget,
                format: agent_output_format_name(format).to_string(),
                git_boost,
                memory_boost,
                pretty: output::json::is_pretty(),
            };
            // Daemon fast path: a running daemon answers from its in-memory
            // index. Embeddings and error-frame requests always load directly.
            if !no_daemon
                && !embeddings
                && !memory_boost
                && error.is_none()
                && let Some(rendered) = try_daemon_agent_context(&path, &request)
            {
                println!("{rendered}");
                return Ok(());
            }
            let error_frames = match error {
                Some(error_path) => {
                    let text = std::fs::read_to_string(&error_path).with_context(|| {
                        format!("failed to read error file {}", error_path.display())
                    })?;
                    query::stacktrace::parse_stack_trace(&text)
                }
                None => Vec::new(),
            };
            let (index, index_load_ms, refreshed) = load_fresh_index_timed(&path)?;
            #[cfg(feature = "embed")]
            let code_embedder = if embeddings && embed_model == EmbedModelArg::Code {
                Some(query::embed::FastembedEmbedder::new_code()?)
            } else {
                None
            };
            #[cfg(feature = "embed")]
            let hybrid = match &code_embedder {
                Some(embedder) => query::HybridOptions::with_embedder(true, embedder),
                None => query::HybridOptions::embeddings(embeddings),
            };
            #[cfg(not(feature = "embed"))]
            let hybrid = {
                let _ = embed_model;
                query::HybridOptions::embeddings(embeddings)
            };
            let output = agent_context_output_for_index(
                &path,
                &index,
                &request,
                hybrid,
                &error_frames,
                index_load_ms,
                refreshed,
            )?;
            println!("{}", render_agent_context_output(&output, &request)?);
        }
        Command::Demo { path, task, lsp } => {
            let output = demo(&path, &task, lsp)?;
            output::json::print(&output)?;
        }
        Command::MemoryClear { path } => {
            let memory_path = query::task_memory_path(&path);
            let removed = query::clear_task_memory(&path)?;
            let output = MemoryClearOutput {
                command: "memory-clear",
                root: root_label(&path),
                path: repo_relative_display(&path, &memory_path),
                removed,
            };
            output::json::print(&output)?;
        }
        Command::Benchmark {
            path,
            task,
            limit,
            snippets_per_file,
            no_snippets,
            profile,
            token_budget,
        } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::benchmark_context_with_options(
                &path,
                &index,
                &task,
                limit,
                snippets_per_file,
                !no_snippets,
                query::ContextViewOptions {
                    profile: profile.into(),
                    token_budget,
                    include_git: false,
                    include_call_paths: false,
                },
            )?;
            output::json::print(&output)?;
        }
        Command::BenchmarkSuite {
            path,
            tasks,
            limit,
            snippets_per_file,
            no_snippets,
            profile,
            token_budget,
        } => {
            let index = store::json_store::load_index(&path)?;
            let tasks_json = fs::read_to_string(&tasks)
                .with_context(|| format!("failed to read benchmark suite: {}", tasks.display()))?;
            let suite: query::BenchmarkSuiteInput = serde_json::from_str(&tasks_json)
                .with_context(|| format!("failed to parse benchmark suite: {}", tasks.display()))?;
            let output = query::benchmark_suite_with_options(
                &path,
                &index,
                suite,
                limit,
                snippets_per_file,
                !no_snippets,
                query::ContextViewOptions {
                    profile: profile.into(),
                    token_budget,
                    include_git: false,
                    include_call_paths: false,
                },
            )?;
            output::json::print(&output)?;
        }
        Command::EvalRetrieval {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
            profile,
            token_budget,
            json: _,
        } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!(
                    "failed to read retrieval eval manifest: {}",
                    manifest.display()
                )
            })?;
            let manifest_value: serde_json::Value = serde_json::from_str(&manifest_json)
                .with_context(|| {
                    format!(
                        "failed to parse retrieval eval manifest: {}",
                        manifest.display()
                    )
                })?;
            let path = retrieval_manifest_root(&manifest_value);
            let index = store::json_store::load_index(&path)?;
            let suite: query::BenchmarkSuiteInput = serde_json::from_value(manifest_value)
                .with_context(|| {
                    format!(
                        "failed to parse retrieval eval tasks: {}",
                        manifest.display()
                    )
                })?;
            let output = query::eval_retrieval_with_options(
                &path,
                &index,
                suite,
                limit,
                snippets_per_file,
                !no_snippets,
                query::ContextViewOptions {
                    profile: profile.into(),
                    token_budget,
                    include_git: false,
                    include_call_paths: false,
                },
            )?;
            let failed = output.failed();
            output::json::print(&output)?;
            if failed {
                std::process::exit(1);
            }
        }
        Command::PerfReport {
            path,
            tasks,
            iterations,
            json: _,
        } => {
            let output = perf_report(&path, tasks.as_deref(), iterations)?;
            output::json::print(&output)?;
        }
        Command::TraceSummary { trace } => {
            let trace_json = fs::read_to_string(&trace)
                .with_context(|| format!("failed to read trace: {}", trace.display()))?;
            let output = query::trace_summary_from_str(&trace_json)
                .with_context(|| format!("failed to summarize trace: {}", trace.display()))?;
            output::json::print(&output)?;
        }
        Command::SessionStart {
            path,
            task,
            client,
            model,
            trace,
            expected_files,
            critical_files,
        } => {
            let output = session_start(
                &path,
                &task,
                client,
                &model,
                &trace,
                expected_files,
                critical_files,
            )?;
            output::json::print(&output)?;
        }
        Command::SessionEvent {
            trace,
            event_command,
            files_read,
            context_selected_files,
            tokens,
            phase,
        } => {
            let output = session_event_with_token_evidence(
                &trace,
                &event_command,
                files_read,
                context_selected_files,
                tokens,
                phase,
                None,
            )?;
            output::json::print(&output)?;
        }
        Command::SessionFinish {
            trace,
            out,
            ground_truth_files,
        } => {
            let output = session_finish(&trace, &out, &ground_truth_files)?;
            output::json::print(&output)?;
        }
        Command::TraceReplay {
            path,
            tasks,
            output,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let index = store::json_store::load_index(&path)?;
            let tasks_json = fs::read_to_string(&tasks)
                .with_context(|| format!("failed to read benchmark suite: {}", tasks.display()))?;
            let suite: query::BenchmarkSuiteInput = serde_json::from_str(&tasks_json)
                .with_context(|| format!("failed to parse benchmark suite: {}", tasks.display()))?;
            let replay =
                query::trace_replay(&path, &index, suite, limit, snippets_per_file, !no_snippets)?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).with_context(|| {
                    format!("failed to create output directory: {}", parent.display())
                })?;
            }
            let replay_json = serde_json::to_string_pretty(&replay)?;
            fs::write(&output, replay_json)
                .with_context(|| format!("failed to write trace replay: {}", output.display()))?;
            output::json::print(&replay)?;
        }
        Command::TraceCheck { trace, strict } => {
            let trace_json = fs::read_to_string(&trace)
                .with_context(|| format!("failed to read trace: {}", trace.display()))?;
            let output = if strict {
                query::trace_check_from_str_with_options(&trace_json, true)
            } else {
                query::trace_check_from_str(&trace_json)
            }
            .with_context(|| format!("failed to check trace: {}", trace.display()))?;
            output::json::print(&output)?;
        }
        Command::BenchmarkReport {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!("failed to read benchmark manifest: {}", manifest.display())
            })?;
            let manifest: query::BenchmarkReportManifest = serde_json::from_str(&manifest_json)
                .with_context(|| {
                    format!("failed to parse benchmark manifest: {}", manifest.display())
                })?;
            let output = query::benchmark_report(manifest, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::CompetitiveReport {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let output =
                competitive_report_file(&manifest, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::PublicProofReport {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let output =
                public_proof_report_file(&manifest, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::RepoPackBaseline {
            path,
            id,
            tool,
            out,
        } => {
            let output = repo_pack_baseline(&path, id, tool)?;
            if let Some(out) = out {
                write_json_file(&out, &output)?;
            }
            output::json::print(&output)?;
        }
        Command::AgentNativeBaseline {
            tasks,
            id,
            tool,
            k,
            measurement_command,
            source_artifacts,
            measurement_notes,
            out,
        } => {
            let output = agent_native_baseline(
                &tasks,
                id,
                tool,
                k,
                measurement_command,
                source_artifacts,
                measurement_notes,
            )?;
            if let Some(out) = out {
                write_json_file(&out, &output)?;
            }
            output::json::print(&output)?;
        }
        Command::AgentNativeCheck {
            tasks,
            mode,
            source_artifacts,
            out,
        } => {
            let output = agent_native_check(&tasks, mode, source_artifacts)?;
            if let Some(out) = out {
                write_json_file(&out, &output)?;
            }
            output::json::print(&output)?;
        }
        Command::AgentNativeProtocol { out } => {
            let output = agent_native_protocol_output();
            if let Some(out) = out {
                write_json_file(&out, &output)?;
            }
            output::json::print(&output)?;
        }
        Command::AgentNativeTemplate {
            path,
            tasks,
            k,
            limit,
            snippets_per_file,
            no_snippets,
            profile,
            token_budget,
            out,
        } => {
            let output = agent_native_template(
                &path,
                &tasks,
                AgentNativeTemplateOptions {
                    k,
                    limit,
                    snippets_per_file,
                    include_snippets: !no_snippets,
                    profile,
                    token_budget,
                },
            )?;
            if let Some(out) = out {
                write_json_file(&out, &output)?;
            }
            output::json::print(&output)?;
        }
        Command::BenchmarkDoctor { manifest } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!("failed to read benchmark manifest: {}", manifest.display())
            })?;
            let output = query::benchmark_doctor_from_str(&manifest_json).with_context(|| {
                format!("failed to check benchmark manifest: {}", manifest.display())
            })?;
            output::json::print(&output)?;
        }
        Command::ProofRehearsal {
            preflight,
            fix,
            resume,
            collect_ollama,
            ollama_manifest,
            ollama_model,
            ollama_limit,
            ollama_context_limit,
            retry_count,
            ledger,
        } => {
            let output = proof_rehearsal(ProofRehearsalOptions {
                preflight,
                fix,
                resume,
                collect_ollama,
                ollama_manifest,
                ollama_model,
                ollama_limit,
                ollama_context_limit,
                retry_count,
                ledger,
            })?;
            let failed = output.status == "fail";
            output::json::print(&output)?;
            if failed {
                std::process::exit(1);
            }
        }
        Command::SetupObservedCodexOss50 {
            manifest,
            bootstrap_repos,
            force,
            skip_repo_check,
        } => {
            let output =
                setup_observed_codex_oss_50(&manifest, bootstrap_repos, force, skip_repo_check)?;
            output::json::print(&output)?;
        }
        Command::SetupObservedClaudeOss50 {
            manifest,
            model,
            bootstrap_repos,
            force,
            skip_repo_check,
        } => {
            let output = setup_observed_claude_oss_50(
                &manifest,
                &model,
                bootstrap_repos,
                force,
                skip_repo_check,
            )?;
            output::json::print(&output)?;
        }
        Command::RecordObservedSession {
            manifest,
            client,
            model,
            task_id,
            mode,
            event_command,
            tokens,
            usage_json,
            files_read,
            context_selected_files,
            dry_run,
        } => {
            let output = record_observed_session(
                "record-observed-session",
                &manifest,
                Some(client),
                model.as_deref(),
                &task_id,
                mode,
                &event_command,
                tokens,
                usage_json.as_deref(),
                files_read,
                context_selected_files,
                dry_run,
            )?;
            output::json::print(&output)?;
        }
        Command::CollectClaudeObservedSession {
            manifest,
            task_id,
            mode,
            model,
            max_budget_usd,
            artifact,
            context_limit,
            snippets_per_file,
            allowed_tools,
            dry_run,
        } => {
            let output = collect_claude_observed_session(
                &manifest,
                &task_id,
                mode,
                &model,
                &max_budget_usd,
                artifact.as_deref(),
                context_limit,
                snippets_per_file,
                allowed_tools,
                dry_run,
            )?;
            output::json::print(&output)?;
        }
        Command::ProofSprint { command } => match command {
            ProofSprintCommand::Init {
                manifest,
                client,
                sessions,
                model,
                force,
                skip_repo_check,
            } => {
                let output =
                    proof_sprint_init(&manifest, client, sessions, &model, force, skip_repo_check)?;
                output::json::print(&output)?;
            }
            ProofSprintCommand::Status { manifest, json: _ } => {
                let output = proof_sprint_status(&manifest)?;
                output::json::print(&output)?;
            }
            ProofSprintCommand::Collect {
                manifest,
                task_id,
                mode,
                max_budget_usd,
                dry_run,
            } => {
                let output =
                    proof_sprint_collect(&manifest, &task_id, mode, &max_budget_usd, dry_run)?;
                output::json::print(&output)?;
            }
            ProofSprintCommand::Run {
                manifest,
                max_budget_usd,
                resume,
                limit,
                dry_run,
            } => {
                let output = proof_sprint_run(&manifest, &max_budget_usd, resume, limit, dry_run)?;
                output::json::print(&output)?;
            }
            ProofSprintCommand::Finalize {
                manifest,
                out,
                limit,
            } => {
                let output = proof_sprint_finalize(&manifest, &out, limit)?;
                output::json::print(&output)?;
            }
        },
        Command::RecordCodexObservedSession {
            manifest,
            task_id,
            mode,
            event_command,
            tokens,
            files_read,
            context_selected_files,
            dry_run,
        } => {
            let output = record_codex_observed_session(
                &manifest,
                &task_id,
                mode,
                &event_command,
                tokens,
                files_read,
                context_selected_files,
                dry_run,
            )?;
            output::json::print(&output)?;
        }
        Command::PilotInit { manifest, sessions } => {
            let output = pilot_init(&manifest, sessions)?;
            output::json::print(&output)?;
        }
        Command::PilotTask { command } => match command {
            PilotTaskCommand::Add {
                manifest,
                repo,
                task,
                id,
                expected_files,
                critical_files,
                external,
                client,
                model,
                suite_path,
                pair_id,
                task_category,
                difficulty,
                condition,
                token_accounting_source,
            } => {
                let output = pilot_task_add(
                    &manifest,
                    &repo,
                    &task,
                    id,
                    expected_files,
                    critical_files,
                    external,
                    client,
                    &model,
                    suite_path,
                    pair_id,
                    task_category,
                    difficulty,
                    condition,
                    token_accounting_source,
                )?;
                output::json::print(&output)?;
            }
            PilotTaskCommand::Reject {
                manifest,
                task_id,
                reason,
            } => {
                let output = pilot_task_reject(&manifest, &task_id, &reason)?;
                output::json::print(&output)?;
            }
        },
        Command::PilotRun {
            manifest,
            task_id,
            mode,
            event_command,
            files_read,
            context_selected_files,
            tokens,
        } => {
            let output = pilot_run(
                &manifest,
                &task_id,
                mode,
                &event_command,
                files_read,
                context_selected_files,
                tokens,
            )?;
            output::json::print(&output)?;
        }
        Command::PilotCollectOllama {
            manifest,
            model,
            limit,
            context_limit,
            snippets_per_file,
            baseline_file_limit,
            baseline_line_limit,
        } => {
            let output = pilot_collect_ollama(
                &manifest,
                &model,
                limit,
                context_limit,
                snippets_per_file,
                baseline_file_limit,
                baseline_line_limit,
            )?;
            output::json::print(&output)?;
        }
        Command::PilotCollectLmStudio {
            manifest,
            model,
            base_url,
            limit,
            context_limit,
            snippets_per_file,
            baseline_file_limit,
            baseline_line_limit,
            max_tokens,
        } => {
            let output = pilot_collect_lm_studio(
                &manifest,
                &model,
                &base_url,
                limit,
                context_limit,
                snippets_per_file,
                baseline_file_limit,
                baseline_line_limit,
                max_tokens,
            )?;
            output::json::print(&output)?;
        }
        Command::PilotQa { manifest } => {
            let output = pilot_qa(&manifest)?;
            output::json::print(&output)?;
        }
        Command::PilotFinalize {
            manifest,
            out,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let output = pilot_finalize(&manifest, &out, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::PilotReport {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!("failed to read pilot manifest: {}", manifest.display())
            })?;
            let manifest: query::BenchmarkReportManifest = serde_json::from_str(&manifest_json)
                .with_context(|| {
                    format!("failed to parse pilot manifest: {}", manifest.display())
                })?;
            let output = query::pilot_report(manifest, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::ProofReport {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!("failed to read proof manifest: {}", manifest.display())
            })?;
            let manifest: query::BenchmarkReportManifest = serde_json::from_str(&manifest_json)
                .with_context(|| {
                    format!("failed to parse proof manifest: {}", manifest.display())
                })?;
            let output = query::proof_report(manifest, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::EnterpriseProofReport {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!(
                    "failed to read enterprise proof manifest: {}",
                    manifest.display()
                )
            })?;
            let manifest: query::BenchmarkReportManifest = serde_json::from_str(&manifest_json)
                .with_context(|| {
                    format!(
                        "failed to parse enterprise proof manifest: {}",
                        manifest.display()
                    )
                })?;
            let output =
                query::enterprise_proof_report(manifest, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::PilotDoctor { manifest } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!("failed to read pilot manifest: {}", manifest.display())
            })?;
            let output = query::pilot_doctor_from_str(&manifest_json).with_context(|| {
                format!("failed to check pilot manifest: {}", manifest.display())
            })?;
            output::json::print(&output)?;
        }
        Command::EvidencePack {
            manifest,
            anonymize,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let manifest_json = fs::read_to_string(&manifest).with_context(|| {
                format!("failed to read evidence manifest: {}", manifest.display())
            })?;
            let manifest: query::BenchmarkReportManifest = serde_json::from_str(&manifest_json)
                .with_context(|| {
                    format!("failed to parse evidence manifest: {}", manifest.display())
                })?;
            let pilot = query::pilot_report(manifest, limit, snippets_per_file, !no_snippets)?;
            let mut evidence = serde_json::to_value(pilot)?;
            if anonymize {
                anonymize_evidence(&mut evidence);
            }
            let output = EvidencePackOutput {
                command: "evidence-pack",
                anonymized: anonymize,
                generated_at: now_unix_seconds(),
                protocol: evidence_pack_protocol(&manifest_json),
                evidence,
            };
            output::json::print(&output)?;
        }
        Command::PolicyCheck { trace, strict } => {
            let trace_json = fs::read_to_string(&trace)
                .with_context(|| format!("failed to read trace: {}", trace.display()))?;
            let check = if strict {
                query::trace_check_from_str_with_options(&trace_json, true)
            } else {
                query::trace_check_from_str(&trace_json)
            }
            .with_context(|| format!("failed to check trace: {}", trace.display()))?;
            let check_value = serde_json::to_value(check)?;
            let failed = check_value
                .get("status")
                .and_then(serde_json::Value::as_str)
                == Some("fail");
            let output = PolicyCheckOutput {
                command: "policy-check",
                trace: trace.display().to_string(),
                check: check_value,
            };
            output::json::print(&output)?;
            if failed {
                std::process::exit(1);
            }
        }
        Command::Mcp => {
            crate::mcp::run()?;
        }
        Command::McpConfig { path, format } => {
            let output = mcp_config_output(&path, format);
            output::json::print(&output)?;
        }
        Command::McpRegistryManifest { out } => {
            let manifest = mcp_registry_manifest();
            if let Some(out) = out {
                if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("failed to create {}", parent.display()))?;
                }
                fs::write(&out, serde_json::to_string_pretty(&manifest)?)
                    .with_context(|| format!("failed to write {}", out.display()))?;
            }
            output::json::print(&manifest)?;
        }
        Command::McpContract { out } => {
            let output = mcp_contract_output();
            if let Some(out) = out {
                if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("failed to create {}", parent.display()))?;
                }
                fs::write(&out, serde_json::to_string_pretty(&output)?)
                    .with_context(|| format!("failed to write {}", out.display()))?;
            }
            output::json::print(&output)?;
        }
        Command::Status { path } => {
            let index = store::json_store::load_index(&path).ok();
            let output = query::index_status(&path, index.as_ref());
            output::json::print(&output)?;
        }
        Command::Daemon {
            path,
            lsp,
            interval_ms,
            foreground,
            background: _,
            once,
        } => {
            let output = run_daemon(&path, lsp, interval_ms, foreground, once)?;
            output::json::print(&output)?;
        }
        Command::DaemonStatus { path } => {
            let output = DaemonOutput {
                command: "daemon-status",
                state: load_daemon_state(&path).unwrap_or_else(|| missing_daemon_state(&path)),
            };
            output::json::print(&output)?;
        }
        Command::DaemonStop { path } => {
            write_daemon_stop(&path)?;
            let mut state = load_daemon_state(&path).unwrap_or_else(|| missing_daemon_state(&path));
            state.status = "stop_requested".to_string();
            save_daemon_state(&path, &state)?;
            output::json::print(&DaemonOutput {
                command: "daemon-stop",
                state,
            })?;
        }
        Command::Watch {
            path,
            debounce_ms,
            foreground,
            lsp,
        } => {
            if foreground {
                loop {
                    let output = refresh_watch_index(&path, "watching", "foreground", lsp)?;
                    output::json::print(&output)?;
                    thread::sleep(Duration::from_millis(debounce_ms));
                }
            } else {
                let output = refresh_watch_index(&path, "refreshed", "single_refresh", lsp)?;
                output::json::print(&output)?;
            }
        }
        Command::AgentSetup {
            path,
            client,
            force,
        }
        | Command::SetupAgent {
            client,
            path,
            force,
        } => {
            let output = setup_agent(client, &path, force)?;
            output::json::print(&output)?;
        }
        Command::SetupAuto {
            path,
            force,
            dry_run,
        } => {
            let output = setup_auto(&path, force, dry_run)?;
            output::json::print(&output)?;
        }
        Command::IndexExport { path, out } => {
            let output = index_export(&path, &out)?;
            output::json::print(&output)?;
        }
        Command::IndexImport {
            path,
            from,
            allow_partial,
        } => {
            let output = index_import(&path, &from, allow_partial)?;
            output::json::print(&output)?;
        }
        Command::MemoryExport { path, out } => {
            let (serialized, entries) = query::export_task_memory(&path)?;
            if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(&out, serialized)
                .with_context(|| format!("failed to write {}", out.display()))?;
            output::json::print(&serde_json::json!({
                "command": "memory-export",
                "root": root_label(&path),
                "out": out.display().to_string(),
                "entries": entries
            }))?;
        }
        Command::MemoryImport { path, from } => {
            let raw = fs::read_to_string(&from)
                .with_context(|| format!("failed to read {}", from.display()))?;
            let (imported, total) = query::merge_task_memory(&path, &raw)?;
            output::json::print(&serde_json::json!({
                "command": "memory-import",
                "root": root_label(&path),
                "from": from.display().to_string(),
                "imported": imported,
                "entries_total": total
            }))?;
        }
        Command::Receipt {
            path,
            session,
            latest: _,
            format,
        } => {
            let receipt = session_receipt(&path, session.as_deref())?;
            match format {
                ReceiptFormat::Json => output::json::print(&receipt)?,
                ReceiptFormat::Markdown => println!("{}", receipt_markdown(&receipt)),
            }
        }
        Command::Receipts { path } => {
            let output = session_receipts_rollup(&path)?;
            output::json::print(&output)?;
        }
        Command::Bootstrap {
            path,
            client,
            strict,
            force,
            lsp,
        } => {
            let output = bootstrap(&path, client, strict, force, lsp)?;
            output::json::print(&output)?;
        }
        Command::Doctor {
            path,
            client,
            fix,
            strict,
        } => {
            let output = doctor(&path, client, fix, strict)?;
            output::json::print(&output)?;
        }
        Command::CodexBootstrap { path, model, force } => {
            let output = codex_bootstrap(&path, &model, force)?;
            output::json::print(&output)?;
        }
        Command::CodexHooks { command } => match command {
            CodexHooksCommand::Install {
                path,
                strict,
                force,
                limit,
                snippets_per_file,
                lsp,
            } => {
                let output =
                    codex_hooks_install(&path, strict, force, limit, snippets_per_file, lsp)?;
                output::json::print(&output)?;
            }
            CodexHooksCommand::Doctor {
                path,
                strict,
                smoke,
                fix,
            } => {
                let output = codex_hooks_doctor_with_options(
                    &path,
                    strict,
                    CodexHooksDoctorOptions { smoke, fix },
                );
                output::json::print(&output)?;
            }
            CodexHooksCommand::TrustAck { path } => {
                let output = codex_hooks_trust_ack(&path)?;
                output::json::print(&output)?;
            }
            CodexHooksCommand::Uninstall { path } => {
                let output = codex_hooks_uninstall(&path)?;
                output::json::print(&output)?;
            }
        },
        Command::CodexHook { command } => {
            let output = match command {
                CodexHookCommand::UserPromptSubmit {
                    path,
                    strict,
                    limit,
                    snippets_per_file,
                } => codex_hook_user_prompt_submit(&path, strict, limit, snippets_per_file)?,
                CodexHookCommand::PreToolUse { path, strict } => {
                    codex_hook_pre_tool_use(&path, strict)?
                }
                CodexHookCommand::PostToolUse { path, strict } => {
                    codex_hook_post_tool_use(&path, strict)?;
                    return Ok(());
                }
                CodexHookCommand::PermissionRequest { path, strict } => {
                    codex_hook_permission_request(&path, strict)?
                }
                CodexHookCommand::Stop { path, strict } => {
                    codex_hook_stop(&path, strict)?;
                    return Ok(());
                }
            };
            output::json::print(&output)?;
        }
        Command::ClaudeHooks { command } => match command {
            ClaudeHooksCommand::Install {
                path,
                strict,
                force,
                limit,
                snippets_per_file,
                lsp,
            } => {
                let output =
                    claude_hooks_install(&path, strict, force, limit, snippets_per_file, lsp)?;
                output::json::print(&output)?;
            }
            ClaudeHooksCommand::Doctor { path, strict } => {
                let output = claude_hooks_doctor(&path, strict);
                output::json::print(&output)?;
            }
            ClaudeHooksCommand::Uninstall { path } => {
                let output = claude_hooks_uninstall(&path)?;
                output::json::print(&output)?;
            }
        },
        Command::ClaudeHook { command } => {
            let output = match command {
                ClaudeHookCommand::UserPromptSubmit {
                    path,
                    strict,
                    limit,
                    snippets_per_file,
                } => claude_hook_user_prompt_submit(&path, strict, limit, snippets_per_file)?,
                ClaudeHookCommand::PreToolUse { path, strict } => {
                    claude_hook_pre_tool_use(&path, strict)?
                }
                ClaudeHookCommand::PostToolUse { path, strict } => {
                    claude_hook_post_tool_use(&path, strict)?
                }
                ClaudeHookCommand::PermissionRequest { path, strict } => {
                    claude_hook_permission_request(&path, strict)?
                }
                ClaudeHookCommand::Stop { path, strict } => claude_hook_stop(&path, strict)?,
            };
            output::json::print(&output)?;
        }
        Command::CopilotHooks { command } => {
            run_client_hooks_command(HookClient::Copilot, command)?;
        }
        Command::CopilotHook { command } => {
            run_client_hook_command(HookClient::Copilot, command)?;
        }
        Command::OpenCodeHooks { command } => {
            run_client_hooks_command(HookClient::OpenCode, command)?;
        }
        Command::OpenCodeHook { command } => {
            run_client_hook_command(HookClient::OpenCode, command)?;
        }
        Command::AntigravityHooks { command } => {
            run_client_hooks_command(HookClient::Antigravity, command)?;
        }
        Command::AntigravityHook { command } => {
            run_client_hook_command(HookClient::Antigravity, command)?;
        }
        Command::ClineHooks { command } => {
            run_client_hooks_command(HookClient::Cline, command)?;
        }
        Command::ClineHook { command } => {
            run_client_hook_command(HookClient::Cline, command)?;
        }
        Command::EditorHook {
            path,
            editor,
            force,
        } => {
            let output = editor_hook(&path, editor, force)?;
            output::json::print(&output)?;
        }
        Command::Guard {
            path,
            task,
            trace_out,
            limit,
            snippets_per_file,
        } => {
            let (index, index_load_ms, refreshed) = load_fresh_index_timed(&path)?;
            let mut context =
                query::build_context(&path, &index, &task, limit, snippets_per_file, true)?;
            context.add_index_load_time(index_load_ms);
            if refreshed {
                context.add_warning("rebuilt missing or stale CallSieve index before context");
            }
            let trace_path = trace_out
                .as_ref()
                .map(|path| write_guard_trace(path, &task, &context))
                .transpose()?;
            let output = GuardOutput {
                command: "guard",
                root: root_label(&path),
                task,
                policy: "read_first_before_grep; audit broad grep/read with trace-check --strict",
                local_first_expansion: agent_local_first_expansion(&path, &context),
                context,
                trace_event: AuditEvent {
                    tool: "callsieve_guard",
                    policy: "first_codebase_discovery_tool",
                    context_first: true,
                    rg_ran: None,
                    called_at: now_unix_seconds(),
                },
                trace_path,
            };
            output::json::print(&output)?;
        }
        Command::Begin {
            path,
            task,
            client,
            trace_out,
            proof_trace,
            limit,
            snippets_per_file,
        } => {
            let output = begin_task(
                &path,
                &task,
                client,
                trace_out.as_deref(),
                proof_trace,
                limit,
                snippets_per_file,
            )?;
            output::json::print(&output)?;
        }
        Command::CodexSession {
            path,
            task,
            trace_out,
            model,
            expected_files,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let (index, index_load_ms, refreshed) = load_fresh_index_timed(&path)?;
            let include_snippets = !no_snippets;
            let mut context = query::build_context(
                &path,
                &index,
                &task,
                limit,
                snippets_per_file,
                include_snippets,
            )?;
            context.add_index_load_time(index_load_ms);
            if refreshed {
                context.add_warning("rebuilt missing or stale CallSieve index before context");
            }
            let trace = query::codex_session_trace(
                &path,
                &index,
                query::CodexSessionTraceInput {
                    task: task.clone(),
                    model: model.clone(),
                    expected_files,
                    limit,
                    snippets_per_file,
                    include_snippets,
                },
            )?;
            if let Some(parent) = trace_out
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            fs::write(&trace_out, serde_json::to_vec_pretty(&trace)?)
                .with_context(|| format!("failed to write {}", trace_out.display()))?;
            let output = CodexSessionOutput {
                command: "codex-session",
                root: root_label(&path),
                client: "codex-chatgpt",
                model,
                task,
                policy: "codex_chatgpt_context_first; read returned files before broad grep or repeated file reads",
                local_first_expansion: agent_local_first_expansion(&path, &context),
                context,
                trace,
                trace_event: AuditEvent {
                    tool: "callsieve_codex_session",
                    policy: "first_codebase_discovery_tool",
                    context_first: true,
                    rg_ran: None,
                    called_at: now_unix_seconds(),
                },
                trace_path: trace_out.display().to_string(),
            };
            output::json::print(&output)?;
        }
        Command::Enforce {
            path,
            client,
            trace,
            strict,
            require_shim,
        } => {
            let output = enforce_setup(&path, client, trace.as_deref(), strict, require_shim)?;
            output::json::print(&output)?;
        }
        Command::Shim { command } => match command {
            ShimCommand::Install {
                path,
                force,
                strict,
            } => {
                let output = install_shim(&path, force, strict)?;
                output::json::print(&output)?;
            }
            ShimCommand::Doctor { path } => {
                let output = shim_doctor(&path);
                output::json::print(&output)?;
            }
            ShimCommand::Uninstall { path } => {
                let output = uninstall_shim(&path)?;
                output::json::print(&output)?;
            }
        },
        Command::Hook { command } => match command {
            HookCommand::Install {
                path,
                client,
                strict,
                force,
                lsp,
            } => {
                let output = install_hook(&path, client, strict, force, lsp)?;
                output::json::print(&output)?;
            }
            HookCommand::Doctor { path } => {
                let output = hook_doctor(&path);
                output::json::print(&output)?;
            }
            HookCommand::Uninstall { path } => {
                let output = uninstall_hook(&path)?;
                output::json::print(&output)?;
            }
        },
        Command::Grep {
            path,
            pattern,
            run_rg: should_run_rg,
            limit,
            snippets_per_file,
            shim_strict,
            shim_command,
        } => {
            let (index, index_load_ms, refreshed) = load_fresh_index_timed(&path)?;
            let mut context =
                query::build_context(&path, &index, &pattern, limit, snippets_per_file, true)?;
            context.add_index_load_time(index_load_ms);
            if refreshed {
                context.add_warning("rebuilt missing or stale CallSieve index before context");
            }
            let context = compact_context_value_for_prompt(&context)?;
            let shim_event = if shim_strict {
                Some(record_shim_grep_event(
                    &path,
                    shim_command.as_deref(),
                    &pattern,
                )?)
            } else {
                None
            };
            let rg = if should_run_rg {
                Some(run_rg(&path, &pattern)?)
            } else {
                None
            };
            let mut output = serde_json::to_value(GrepOutput {
                command: "grep",
                policy: "callsieve_context_first; rg only runs when --run-rg is set",
                rg_status: if should_run_rg {
                    "context returned first; rg executed after context"
                } else {
                    "context returned first; pass --run-rg to execute rg after context"
                },
                context,
                audit_event: AuditEvent {
                    tool: "callsieve_grep",
                    policy: "context_before_optional_rg",
                    context_first: true,
                    rg_ran: Some(should_run_rg),
                    called_at: now_unix_seconds(),
                },
                rg,
            })?;
            if let Some(shim_event) = shim_event
                && let Some(object) = output.as_object_mut()
            {
                object.insert("shim_event".to_string(), shim_event);
            }
            output::json::print(&output)?;
        }
        Command::ShimRun {
            path,
            tool,
            strict,
            args,
        } => {
            let output = run_shim_command(&path, tool, strict, &args)?;
            output::json::print(&output)?;
        }
        Command::Stats { path } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::stats(&path, &index)?;
            output::json::print(&output)?;
        }
        Command::BenchPublic {
            manifest,
            repos_dir,
            k,
            out,
            embeddings,
            compare,
        } => {
            if embeddings || compare {
                ensure_embeddings_supported()?;
            }
            if compare {
                let report = bench_public::run_compare(&manifest, &repos_dir, k)?;
                let out_path =
                    bench_public::resolve_compare_output_path(&manifest, out.as_deref(), &report)?;
                bench_public::write_compare_report(&out_path, &report)?;
                let summary =
                    bench_public::CompareSummaryOutput::new(&manifest, &out_path, &report);
                output::json::print(&summary)?;
            } else {
                let report = bench_public::run_with_options(
                    &manifest,
                    &repos_dir,
                    k,
                    bench_public::RunOptions::embeddings(embeddings),
                )?;
                let out_path =
                    bench_public::resolve_output_path(&manifest, out.as_deref(), &report)?;
                bench_public::write_report(&out_path, &report)?;
                let summary = bench_public::SummaryOutput::new(&manifest, &out_path, &report);
                output::json::print(&summary)?;
            }
        }
        Command::BenchRun {
            manifest,
            workdir,
            compare,
            k,
            limit,
            out,
            resume,
        } => {
            ensure_bench_run_supported()?;
            if resume && out.is_none() {
                bail!("bench-run --resume requires --out <path>");
            }
            if compare {
                let report = if resume {
                    bench_public::run_bench_compare_resume(
                        &manifest,
                        &workdir,
                        k,
                        limit,
                        out.as_deref().expect("validated --out for --resume"),
                    )?
                } else {
                    bench_public::run_bench_compare(&manifest, &workdir, k, limit)?
                };
                let out_path =
                    bench_public::resolve_compare_output_path(&manifest, out.as_deref(), &report)?;
                bench_public::write_compare_report(&out_path, &report)?;
                let summary = bench_public::CompareSummaryOutput::new_for_command(
                    "bench-run",
                    &manifest,
                    &out_path,
                    &report,
                );
                output::json::print(&summary)?;
            } else {
                let report = if resume {
                    bench_public::run_bench_with_resume(
                        &manifest,
                        &workdir,
                        k,
                        limit,
                        bench_public::RunOptions::default(),
                        out.as_deref().expect("validated --out for --resume"),
                    )?
                } else {
                    bench_public::run_bench(&manifest, &workdir, k, limit)?
                };
                let out_path =
                    bench_public::resolve_output_path(&manifest, out.as_deref(), &report)?;
                bench_public::write_report(&out_path, &report)?;
                let summary = bench_public::SummaryOutput::new_for_command(
                    "bench-run",
                    &manifest,
                    &out_path,
                    &report,
                );
                output::json::print(&summary)?;
            }
        }
    }

    Ok(())
}

fn embeddings_env_enabled() -> bool {
    env::var("CALLSIEVE_EMBEDDINGS")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
}

#[cfg(feature = "embed")]
fn ensure_embeddings_supported() -> Result<()> {
    Ok(())
}

#[cfg(not(feature = "embed"))]
fn ensure_embeddings_supported() -> Result<()> {
    bail!("--embeddings requires building with --features embed");
}

#[cfg(feature = "embed")]
fn ensure_bench_run_supported() -> Result<()> {
    Ok(())
}

#[cfg(not(feature = "embed"))]
fn ensure_bench_run_supported() -> Result<()> {
    bail!("bench-run requires building with --features embed");
}

#[cfg(feature = "embed")]
fn build_embeddings_cache(
    root: &Path,
    index: &store::CodeIndex,
    embed_model: EmbedModelArg,
) -> Result<PathBuf> {
    let embedder = match embed_model {
        EmbedModelArg::Small => query::embed::FastembedEmbedder::new_default()?,
        EmbedModelArg::Code => query::embed::FastembedEmbedder::new_code()?,
    };
    query::embed_build::build_and_write_embeds(root, index, &embedder, true)
}

#[cfg(not(feature = "embed"))]
fn build_embeddings_cache(
    _root: &Path,
    _index: &store::CodeIndex,
    _embed_model: EmbedModelArg,
) -> Result<PathBuf> {
    bail!("--embeddings requires building with --features embed");
}

fn remove_embeddings_cache_if_present(root: &Path) -> Result<()> {
    let path = root.join(".callsieve").join("embeds.bin");
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn run_client_hooks_command(client: HookClient, command: ClientHooksCommand) -> Result<()> {
    match command {
        ClientHooksCommand::Install {
            path,
            strict,
            force,
            limit,
            snippets_per_file,
            lsp,
        } => {
            let output =
                client_hooks_install(&path, client, strict, force, limit, snippets_per_file, lsp)?;
            output::json::print(&output)?;
        }
        ClientHooksCommand::Doctor { path, strict } => {
            let output = client_hooks_doctor(&path, client, strict);
            output::json::print(&output)?;
        }
        ClientHooksCommand::Uninstall { path } => {
            let output = client_hooks_uninstall(&path, client)?;
            output::json::print(&output)?;
        }
    }
    Ok(())
}

fn run_client_hook_command(client: HookClient, command: ClientHookCommand) -> Result<()> {
    let output = match command {
        ClientHookCommand::UserPromptSubmit {
            path,
            strict,
            limit,
            snippets_per_file,
        } => client_hook_user_prompt_submit(&path, client, strict, limit, snippets_per_file)?,
        ClientHookCommand::PreToolUse { path, strict } => {
            client_hook_pre_tool_use(&path, client, strict)?
        }
        ClientHookCommand::PostToolUse { path, strict } => {
            client_hook_post_tool_use(&path, client, strict)?
        }
        ClientHookCommand::PermissionRequest { path, strict } => {
            client_hook_permission_request(&path, client, strict)?
        }
        ClientHookCommand::Stop { path, strict } => client_hook_stop(&path, client, strict)?,
    };
    output::json::print(&output)?;
    Ok(())
}

fn load_index_timed(path: &Path) -> Result<(store::CodeIndex, u64)> {
    let started = Instant::now();
    let index = store::json_store::load_index(path)?;
    Ok((index, duration_ms(started.elapsed())))
}

fn load_fresh_index_timed(path: &Path) -> Result<(store::CodeIndex, u64, bool)> {
    let started = Instant::now();
    let initial_index = store::json_store::load_index(path).ok();
    let initial_status = query::index_status(path, initial_index.as_ref());
    if initial_status.is_fresh() {
        let index = initial_index.expect("fresh status requires an index");
        return Ok((index, duration_ms(started.elapsed()), false));
    }

    let index = indexer::build_index(path).with_context(|| {
        format!(
            "failed to rebuild missing or stale CallSieve index for {}; run `callsieve index {}`",
            path.display(),
            path.display()
        )
    })?;
    store::json_store::save_index(path, &index).with_context(|| {
        format!(
            "failed to save rebuilt CallSieve index for {}; run `callsieve index {}`",
            path.display(),
            path.display()
        )
    })?;
    Ok((index, duration_ms(started.elapsed()), true))
}

fn demo(path: &Path, task: &str, lsp: bool) -> Result<DemoOutput> {
    let index = indexer::build_index_with_options(
        path,
        indexer::IndexOptions {
            lsp,
            ..indexer::IndexOptions::default()
        },
    )?;
    let index_path = store::json_store::save_index(path, &index)?;
    let context = query::build_context(path, &index, task, 8, 2, true)?;
    let benchmark = query::benchmark_context(path, &index, task, 8, 2, true)?;
    let context_payload_reduction = query::benchmark_context_payload_reduction_value(&benchmark)?;

    Ok(DemoOutput {
        command: "demo",
        root: root_label(path),
        task: task.to_string(),
        index: DemoIndexSummary {
            path: repo_relative_display(path, &index_path),
            files: index.files.len(),
            symbols: index.symbols.len(),
            imports: index.imports.len(),
            references: index.references.len(),
            lsp_enriched: index.metadata.lsp_enriched,
        },
        read_first: query::context_read_first_files(&context),
        context_payload_reduction,
        next_commands: vec![
            format!("callsieve agent-context {} {:?}", path.display(), task),
            format!("callsieve setup-auto {} --dry-run", path.display()),
            format!("callsieve mcp-config {} --format json", path.display()),
            "callsieve proof-rehearsal --preflight".to_string(),
        ],
        warnings: index.warnings,
    })
}

fn print_context_output(
    output: &query::ContextOutput,
    format: AgentOutputFormat,
    view_options: query::ContextViewOptions,
) -> Result<()> {
    let value = query::context_value(output, view_options)?;
    match format {
        AgentOutputFormat::Json => output::json::print(&value),
        AgentOutputFormat::Markdown => {
            println!(
                "{}",
                context_markdown(&value, "grep_only_if_context_is_insufficient", None)
            );
            Ok(())
        }
    }
}

/// The full agent-context parameter set, serializable so a running daemon
/// can execute the identical request against its in-memory index.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct AgentContextRequest {
    task: String,
    limit: usize,
    snippets_per_file: usize,
    why_debug: bool,
    profile: String,
    token_budget: usize,
    format: String,
    git_boost: bool,
    #[serde(default)]
    memory_boost: bool,
    pretty: bool,
}

fn profile_arg_name(profile: ContextProfileArg) -> &'static str {
    match profile {
        ContextProfileArg::Skim => "skim",
        ContextProfileArg::Normal => "normal",
        ContextProfileArg::Full => "full",
    }
}

fn parse_profile_name(name: &str) -> query::ContextProfile {
    match name {
        "normal" => query::ContextProfile::Normal,
        "full" => query::ContextProfile::Full,
        _ => query::ContextProfile::Skim,
    }
}

fn agent_output_format_name(format: AgentOutputFormat) -> &'static str {
    match format {
        AgentOutputFormat::Json => "json",
        AgentOutputFormat::Markdown => "markdown",
    }
}

/// Everything agent-context does after the index is available; shared by the
/// direct CLI path and the daemon's socket serving path so both produce
/// identical packets.
fn agent_context_output_for_index(
    root: &Path,
    index: &store::CodeIndex,
    request: &AgentContextRequest,
    hybrid: query::HybridOptions<'_>,
    error_frames: &[query::stacktrace::StackFrame],
    index_load_ms: u64,
    refreshed: bool,
) -> Result<AgentContextOutput> {
    let retrieval_task = effective_task_for_retrieval(root, &request.task);
    let mut context = query::build_context_with(
        root,
        index,
        &retrieval_task,
        query::ContextOptions {
            limit: request.limit,
            snippets_per_file: request.snippets_per_file,
            include_snippets: true,
            why_debug: request.why_debug,
            hybrid,
            error_frames,
            git_boost: request.git_boost,
            memory_boost: request.memory_boost,
        },
    )?;
    context.add_index_load_time(index_load_ms);
    if refreshed {
        context.add_warning("rebuilt missing or stale CallSieve index before context");
    }
    let memory = query::task_memory_for_context(root, &context, now_unix_seconds())?;
    Ok(AgentContextOutput {
        instruction: AgentContextInstruction {
            local_first_expansion: agent_indexed_local_first_expansion(&context),
        },
        memory,
        context,
    })
}

fn render_agent_context_output(
    output: &AgentContextOutput,
    request: &AgentContextRequest,
) -> Result<String> {
    let profile = parse_profile_name(&request.profile);
    let view_options = query::ContextViewOptions {
        profile,
        token_budget: Some(request.token_budget),
        include_git: request.git_boost,
        include_call_paths: false,
    };
    let context = query::context_value(&output.context, view_options)?;
    let mut value = serde_json::json!({
        "instruction": &output.instruction,
        "memory": &output.memory,
        "context": context
    });
    let json_format = request.format != "markdown";
    if json_format && view_options.profile == query::ContextProfile::Skim {
        trim_agent_context_root(&mut value)?;
    }
    compact_agent_memory_envelope(
        &mut value,
        view_options.profile == query::ContextProfile::Skim,
    );
    apply_agent_context_envelope_budget(&mut value, view_options.token_budget)?;
    if json_format {
        output::json::render(&value, request.pretty)
    } else {
        let context = value.get("context").unwrap_or(&value);
        let grep_policy = value
            .get("instruction")
            .and_then(|instruction| instruction.get("grep_policy"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("grep_only_if_context_is_insufficient");
        let local_first_expansion = value.get("instruction").and_then(instruction_expansion);
        Ok(context_markdown(
            context,
            grep_policy,
            local_first_expansion,
        ))
    }
}

fn trim_agent_context_root(value: &mut serde_json::Value) -> Result<()> {
    let Some(context) = value.get_mut("context") else {
        return Ok(());
    };
    if let Some(context) = context.as_object_mut() {
        context.remove("root");
    }
    refresh_agent_context_stats(context)?;
    Ok(())
}

fn apply_agent_context_envelope_budget(
    value: &mut serde_json::Value,
    token_budget: Option<usize>,
) -> Result<()> {
    let Some(token_budget) = token_budget else {
        return Ok(());
    };
    if query::value_estimated_tokens(value)? <= token_budget {
        return Ok(());
    }

    compact_agent_memory(value, false);
    if query::value_estimated_tokens(value)? <= token_budget {
        return Ok(());
    }

    compact_agent_memory(value, true);
    if query::value_estimated_tokens(value)? <= token_budget {
        return Ok(());
    }

    trim_agent_expansion_fields(value, &["inspect_next_files"]);
    if query::value_estimated_tokens(value)? <= token_budget {
        return Ok(());
    }

    trim_agent_expansion_fields(
        value,
        &["grep_fallback", "expand_relationships", "inspect_tests"],
    );
    if query::value_estimated_tokens(value)? <= token_budget {
        return Ok(());
    }

    trim_agent_graph_hints(value)?;
    while query::value_estimated_tokens(value)? > token_budget {
        if !drop_last_agent_context_file(value)? {
            break;
        }
    }
    if query::value_estimated_tokens(value)? > token_budget {
        trim_agent_expansion_fields(value, &["inspect_top_file"]);
    }
    if query::value_estimated_tokens(value)? > token_budget {
        compact_agent_memory_minimal(value);
    }
    if query::value_estimated_tokens(value)? > token_budget {
        trim_context_retrieval_note(value)?;
    }

    Ok(())
}

fn compact_agent_memory_envelope(value: &mut serde_json::Value, skim: bool) {
    let selected_files = value
        .get("context")
        .map(context_read_first_files)
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let selected_symbols = value
        .get("context")
        .map(context_read_first_symbol_names)
        .unwrap_or_default();

    let remove_memory = if let Some(memory) = value
        .get_mut("memory")
        .and_then(serde_json::Value::as_object_mut)
    {
        memory.remove("path");
        memory.remove("policy");
        filter_selected_memory_recommendations(memory, &selected_files, &selected_symbols);
        memory.remove("similar_tasks");
        rename_memory_field(memory, "cache_hit", "hit");
        rename_memory_field(memory, "recommended_files", "f");
        rename_memory_field(memory, "recommended_symbols", "sy");
        if let Some(files) = memory_array_mut(memory, &["f"]) {
            files.truncate(2);
        }
        if let Some(symbols) = memory_array_mut(memory, &["sy"]) {
            symbols.truncate(3);
        }
        if skim {
            memory.remove("f");
            memory.remove("sy");
        }
        remove_empty_memory_arrays(memory);
        skim && memory.keys().all(|key| key == "hit")
    } else {
        false
    };
    if remove_memory && let Some(object) = value.as_object_mut() {
        object.remove("memory");
    }
}

fn rename_memory_field(
    memory: &mut serde_json::Map<String, serde_json::Value>,
    from: &str,
    to: &str,
) {
    if memory.contains_key(to) {
        memory.remove(from);
        return;
    }
    if let Some(value) = memory.remove(from) {
        memory.insert(to.to_string(), value);
    }
}

fn filter_selected_memory_recommendations(
    memory: &mut serde_json::Map<String, serde_json::Value>,
    selected_files: &BTreeSet<String>,
    selected_symbols: &BTreeSet<String>,
) {
    if let Some(files) = memory_array_mut(memory, &["f", "recommended_files"]) {
        files.retain(|file| {
            file.as_str()
                .map(|file| !selected_files.contains(file))
                .unwrap_or(true)
        });
    }
    if let Some(symbols) = memory_array_mut(memory, &["sy", "recommended_symbols"]) {
        symbols.retain(|symbol| {
            symbol
                .as_str()
                .map(|symbol| !selected_symbols.contains(symbol))
                .unwrap_or(true)
        });
    }
}

fn memory_array_mut<'a>(
    memory: &'a mut serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<&'a mut Vec<serde_json::Value>> {
    for key in keys {
        if memory
            .get(*key)
            .and_then(serde_json::Value::as_array)
            .is_some()
        {
            return memory
                .get_mut(*key)
                .and_then(serde_json::Value::as_array_mut);
        }
    }
    None
}

fn remove_empty_memory_arrays(memory: &mut serde_json::Map<String, serde_json::Value>) {
    for field in [
        "similar_tasks",
        "recommended_files",
        "recommended_symbols",
        "sim",
        "f",
        "sy",
    ] {
        if memory
            .get(field)
            .and_then(serde_json::Value::as_array)
            .map(Vec::is_empty)
            .unwrap_or(false)
        {
            memory.remove(field);
        }
    }
}

fn trim_agent_expansion_fields(value: &mut serde_json::Value, fields: &[&str]) {
    if let Some(expansion) = value
        .get_mut("instruction")
        .and_then(instruction_expansion_mut)
        .and_then(serde_json::Value::as_object_mut)
    {
        for field in fields {
            for alias in expansion_field_aliases(field) {
                expansion.remove(*alias);
            }
        }
    }
}

fn instruction_expansion(instruction: &serde_json::Value) -> Option<&serde_json::Value> {
    instruction
        .get("x")
        .or_else(|| instruction.get("local_first_expansion"))
}

fn instruction_expansion_mut(
    instruction: &mut serde_json::Value,
) -> Option<&mut serde_json::Value> {
    if instruction.get("x").is_some() {
        instruction.get_mut("x")
    } else {
        instruction.get_mut("local_first_expansion")
    }
}

fn expansion_field_aliases(field: &str) -> &'static [&'static str] {
    match field {
        "inspect_top_file" | "top" | "o" => &["inspect_top_file", "top", "o"],
        "inspect_next_files" | "next" | "n" => &["inspect_next_files", "next", "n"],
        "expand_relationships" | "rel" | "r" => &["expand_relationships", "rel", "r"],
        "inspect_tests" | "tests" | "t" => &["inspect_tests", "tests", "t"],
        "grep_fallback" | "grep" => &["grep_fallback", "grep"],
        _ => &[],
    }
}

fn trim_agent_graph_hints(value: &mut serde_json::Value) -> Result<()> {
    let Some(context) = value.get_mut("context") else {
        return Ok(());
    };
    let Some(files) = context
        .get_mut("read_first")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(());
    };
    for file in files {
        if let Some(file) = file.as_object_mut() {
            file.remove("g");
            file.remove("cp");
            file.remove("graph_hints");
            file.remove("call_paths");
        }
    }
    refresh_agent_context_stats(context)?;
    Ok(())
}

fn compact_agent_memory(value: &mut serde_json::Value, aggressive: bool) {
    let Some(memory) = value
        .get_mut("memory")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    if let Some(files) = memory_array_mut(memory, &["f", "recommended_files"]) {
        files.truncate(if aggressive { 3 } else { 5 });
    }
    if let Some(symbols) = memory_array_mut(memory, &["sy", "recommended_symbols"]) {
        symbols.truncate(if aggressive { 0 } else { 5 });
    }
    if let Some(tasks) = memory_array_mut(memory, &["sim", "similar_tasks"]) {
        if aggressive {
            tasks.clear();
        } else {
            tasks.truncate(1);
            if let Some(task) = tasks.first_mut().and_then(serde_json::Value::as_object_mut) {
                if let Some(shared_terms) = task
                    .get_mut("shared_terms")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    shared_terms.truncate(4);
                }
                if let Some(files) = task
                    .get_mut("read_first_files")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    files.truncate(4);
                }
            }
        }
    }
    remove_empty_memory_arrays(memory);
}

fn compact_agent_memory_minimal(value: &mut serde_json::Value) {
    let Some(memory) = value
        .get_mut("memory")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    memory.remove("path");
    memory.remove("policy");
    if let Some(files) = memory_array_mut(memory, &["f", "recommended_files"]) {
        files.clear();
    }
    if let Some(symbols) = memory_array_mut(memory, &["sy", "recommended_symbols"]) {
        symbols.clear();
    }
    if let Some(tasks) = memory_array_mut(memory, &["sim", "similar_tasks"]) {
        tasks.clear();
    }
    remove_empty_memory_arrays(memory);
}

fn trim_context_retrieval_note(value: &mut serde_json::Value) -> Result<()> {
    let Some(context) = value.get_mut("context") else {
        return Ok(());
    };
    if let Some(retrieval_cost) = context
        .get_mut("retrieval_cost")
        .and_then(serde_json::Value::as_object_mut)
    {
        retrieval_cost.remove("note");
        refresh_agent_context_stats(context)?;
    }
    Ok(())
}

fn drop_last_agent_context_file(value: &mut serde_json::Value) -> Result<bool> {
    let Some(context) = value.get_mut("context") else {
        return Ok(false);
    };
    let dropped = {
        let Some(files) = context
            .get_mut("read_first")
            .and_then(serde_json::Value::as_array_mut)
        else {
            return Ok(false);
        };
        if files.len() <= 1 {
            false
        } else {
            files.pop();
            true
        }
    };
    if dropped {
        refresh_agent_context_stats(context)?;
    }
    Ok(dropped)
}

fn refresh_agent_context_stats(context: &mut serde_json::Value) -> Result<()> {
    query::trim_selection_summary_to_read_first(context);
    let (selected_files, selected_symbols, related_tests) = context
        .get("read_first")
        .and_then(serde_json::Value::as_array)
        .map(|files| {
            let selected_symbols = files
                .iter()
                .filter_map(|file| json_array_any(file, &["sy", "symbols"]))
                .map(Vec::len)
                .sum::<usize>();
            let related_tests = files
                .iter()
                .map(|file| {
                    file.get("related_tests")
                        .and_then(serde_json::Value::as_array)
                        .map(Vec::len)
                        .or_else(|| {
                            file.get("i")
                                .or_else(|| file.get("impact"))
                                .and_then(compact_or_legacy_impact_tests_len)
                        })
                        .unwrap_or_default()
                })
                .sum::<usize>();
            (files.len(), selected_symbols, related_tests)
        })
        .unwrap_or_default();

    if let Some(stats) = context
        .get_mut("stats")
        .and_then(serde_json::Value::as_object_mut)
    {
        if stats.contains_key("tokens") || stats.contains_key("local") {
            stats.remove("tokens");
            stats.remove("t");
        } else {
            stats.insert(
                "selected_files".to_string(),
                serde_json::json!(selected_files),
            );
            stats.insert(
                "selected_symbols".to_string(),
                serde_json::json!(selected_symbols),
            );
            stats.insert(
                "related_tests".to_string(),
                serde_json::json!(related_tests),
            );
            stats.remove("estimated_tokens");
        }
    }
    let estimated_tokens = query::value_estimated_tokens(context)?;
    if let Some(stats) = context
        .get_mut("stats")
        .and_then(serde_json::Value::as_object_mut)
    {
        if stats.contains_key("local") {
            stats.insert("t".to_string(), serde_json::json!(estimated_tokens));
        } else {
            stats.insert(
                "estimated_tokens".to_string(),
                serde_json::json!(estimated_tokens),
            );
        }
    }
    Ok(())
}

fn agent_local_first_expansion(
    _path: &Path,
    context: &query::ContextOutput,
) -> AgentLocalFirstExpansion {
    let targets = query::context_read_first_targets(context);
    let top_target = targets.first().cloned();
    let top_target_is_code = top_target.as_ref().is_some_and(|target| target.is_code);
    let inspect_next_files = targets
        .iter()
        .skip(1)
        .take(MAX_LOCAL_EXPANSION_NEXT_FILES)
        .map(agent_focus_expansion)
        .collect();

    AgentLocalFirstExpansion {
        inspect_top_file: top_target.as_ref().map(agent_focus_expansion),
        inspect_next_files,
        expand_relationships: top_target_is_code,
        inspect_tests: top_target_is_code,
    }
}

fn agent_indexed_local_first_expansion(
    context: &query::ContextOutput,
) -> AgentIndexedLocalFirstExpansion {
    let targets = query::context_read_first_targets(context);
    let top_target_is_code = targets.first().is_some_and(|target| target.is_code);
    let inspect_next_indexes = targets
        .iter()
        .enumerate()
        .skip(1)
        .take(MAX_LOCAL_EXPANSION_NEXT_FILES)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let (inspect_next_file, inspect_next_files) = match inspect_next_indexes.as_slice() {
        [index] => (Some(*index), Vec::new()),
        _ => (None, inspect_next_indexes),
    };

    AgentIndexedLocalFirstExpansion {
        inspect_top_file: (!targets.is_empty()).then_some(0),
        inspect_next_file,
        inspect_next_files,
        expand_relationships: top_target_is_code.then_some(1),
        inspect_tests: top_target_is_code.then_some(1),
    }
}

fn local_first_expansion_for_context_value(
    _root: &Path,
    context: &serde_json::Value,
) -> Option<serde_json::Value> {
    let targets = context_value_read_first_targets(context);
    let top_target = targets.first()?;
    let inspect_next_files = targets
        .iter()
        .skip(1)
        .take(MAX_LOCAL_EXPANSION_NEXT_FILES)
        .map(agent_focus_expansion_value)
        .collect::<Vec<_>>();
    let mut expansion = serde_json::json!({
        "o": agent_focus_expansion_value(top_target)
    });
    if top_target.is_code
        && let Some(object) = expansion.as_object_mut()
    {
        object.insert("rel".to_string(), serde_json::json!(true));
        object.insert("tests".to_string(), serde_json::json!(true));
    }
    if !inspect_next_files.is_empty()
        && let Some(object) = expansion.as_object_mut()
    {
        object.insert("next".to_string(), serde_json::json!(inspect_next_files));
    }
    Some(expansion)
}

fn agent_focus_expansion(target: &query::FocusTarget) -> AgentFocusExpansion {
    AgentFocusExpansion {
        file: target.file.clone(),
        symbol: target.symbol.clone(),
        line: target.line,
    }
}

fn agent_focus_expansion_value(target: &query::FocusTarget) -> serde_json::Value {
    serde_json::to_value(agent_focus_expansion(target)).unwrap_or_else(|_| serde_json::json!({}))
}

fn skip_false(value: &bool) -> bool {
    !*value
}

fn compact_context_value_for_prompt(context: &query::ContextOutput) -> Result<serde_json::Value> {
    query::context_value(
        context,
        query::ContextViewOptions {
            profile: query::ContextProfile::Skim,
            token_budget: Some(query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET),
            include_git: false,
            include_call_paths: false,
        },
    )
}

fn context_markdown(
    context: &serde_json::Value,
    grep_policy: &str,
    local_first_expansion: Option<&serde_json::Value>,
) -> String {
    let task = json_string(context, &["task"]).unwrap_or_default();
    let root = json_string(context, &["root"]).unwrap_or_default();
    let mut output = String::new();
    output.push_str("# CallSieve Context\n\n");
    if !task.is_empty() {
        output.push_str(&format!("Task: {task}\n"));
    }
    if !root.is_empty() {
        output.push_str(&format!("Root: {root}\n"));
    }
    output.push_str(&format!("Grep policy: {grep_policy}\n\n"));
    if let Some(tokens) = json_usize(context, &["retrieval_cost", "retrieval_model_tokens"]) {
        output.push_str(&format!(
            "Retrieval cost: {tokens} AI model tokens for local retrieval. Returned context still counts when read.\n\n"
        ));
    }
    if let Some(estimated) = json_usize(context, &["stats", "t"])
        .or_else(|| json_usize(context, &["stats", "tokens"]))
        .or_else(|| json_usize(context, &["stats", "estimated_tokens"]))
    {
        match json_usize(context, &["stats", "b"])
            .or_else(|| json_usize(context, &["stats", "budget"]))
            .or_else(|| json_usize(context, &["stats", "token_budget"]))
        {
            Some(budget) => {
                output.push_str(&format!(
                    "Packet estimate: {estimated}/{budget} tokens.\n\n"
                ));
            }
            None => {
                output.push_str(&format!("Packet estimate: {estimated} tokens.\n\n"));
            }
        }
    }
    if let Some(expansion) = local_first_expansion {
        output.push_str("## Local Expansion\n");
        if let Some(policy) = expansion.get("policy").and_then(serde_json::Value::as_str) {
            output.push_str(&format!("Policy: {policy}\n"));
        }
        if let Some(command) =
            expansion_command(expansion, "inspect_top_file", &root, Some(context))
        {
            output.push_str(&format!("- `{command}`\n"));
        }
        for command in expansion_commands(expansion, "inspect_next_files", &root, Some(context)) {
            output.push_str(&format!("- `{command}`\n"));
        }
        for key in ["expand_relationships", "inspect_tests", "grep_fallback"] {
            if let Some(command) = expansion_command(expansion, key, &root, Some(context)) {
                output.push_str(&format!("- `{command}`\n"));
            }
        }
        output.push('\n');
    }
    output.push_str("## Read First\n");

    let files = context
        .get("read_first")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if files.is_empty() {
        output.push_str("\nNo read-first files selected.\n");
        return output;
    }

    for (index, file) in files.into_iter().enumerate() {
        let rank = json_usize(&file, &["rank"]).unwrap_or(index + 1);
        let path =
            json_string_any(&file, &["f", "file"]).unwrap_or_else(|| "<unknown>".to_string());
        let language = json_string(&file, &["language"]).unwrap_or_default();
        let score = json_usize_any(&file, &["s", "score"]).unwrap_or_default();
        output.push_str(&format!("\n{rank}. `{path}`"));
        if !language.is_empty() {
            output.push_str(&format!(" ({language}, score {score})"));
        } else if score > 0 {
            output.push_str(&format!(" (score {score})"));
        }
        output.push('\n');

        if let Some(why) = json_array_any(&file, &["w", "why"])
            && !why.is_empty()
        {
            output.push_str("   Why: ");
            output.push_str(
                &why.iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; "),
            );
            output.push('\n');
        }

        if let Some(symbols) = json_array_any(&file, &["sy", "symbols"])
            && !symbols.is_empty()
        {
            let names = symbols
                .iter()
                .filter_map(|symbol| {
                    let name = symbol_name_from_value(symbol)?;
                    let kind = symbol_kind_from_value(symbol).unwrap_or("function");
                    Some(format!("{kind} `{name}`"))
                })
                .collect::<Vec<_>>();
            if !names.is_empty() {
                output.push_str(&format!("   Symbols: {}\n", names.join(", ")));
            }
        }

        let graph_hint_parts = markdown_graph_hints(&file);
        if !graph_hint_parts.is_empty() {
            output.push_str(&format!(
                "   Graph hints: {}\n",
                graph_hint_parts.join("; ")
            ));
        }
        let call_path_parts = markdown_call_paths(&file);
        if !call_path_parts.is_empty() {
            output.push_str(&format!("   Call paths: {}\n", call_path_parts.join("; ")));
        }

        if let Some(tests) = file
            .get("related_tests")
            .and_then(serde_json::Value::as_array)
            && !tests.is_empty()
        {
            let test_files = tests
                .iter()
                .filter_map(|test| test.get("file").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>();
            if !test_files.is_empty() {
                output.push_str(&format!("   Related tests: {}\n", test_files.join(", ")));
            }
        }

        if let Some(snippets) = file.get("snippets").and_then(serde_json::Value::as_array) {
            for snippet in snippets {
                let start = snippet
                    .get("lines")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|lines| lines.first())
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                let end = snippet
                    .get("lines")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|lines| lines.get(1))
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(start);
                let text = snippet
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if !text.is_empty() {
                    output.push_str(&format!("   Snippet lines {start}-{end}:\n"));
                    for line in text.lines() {
                        output.push_str("      ");
                        output.push_str(line);
                        output.push('\n');
                    }
                }
            }
        }
    }

    output
}

fn expansion_command_value<'a>(
    expansion: &'a serde_json::Value,
    field: &str,
) -> Option<&'a serde_json::Value> {
    expansion_field_aliases(field)
        .iter()
        .find_map(|alias| expansion.get(*alias))
}

fn expansion_commands(
    expansion: &serde_json::Value,
    field: &str,
    root: &str,
    context: Option<&serde_json::Value>,
) -> Vec<String> {
    let Some(value) = expansion_command_value(expansion, field) else {
        return Vec::new();
    };
    if let Some(values) = value.as_array() {
        return values
            .iter()
            .filter_map(|value| {
                expansion_command_from_value(expansion, field, value, root, context)
            })
            .collect();
    }
    expansion_command_from_value(expansion, field, value, root, context)
        .into_iter()
        .collect()
}

fn expansion_command(
    expansion: &serde_json::Value,
    field: &str,
    root: &str,
    context: Option<&serde_json::Value>,
) -> Option<String> {
    let value = expansion_command_value(expansion, field)?;
    expansion_command_from_value(expansion, field, value, root, context)
}

fn expansion_command_from_value(
    expansion: &serde_json::Value,
    field: &str,
    value: &serde_json::Value,
    root: &str,
    context: Option<&serde_json::Value>,
) -> Option<String> {
    if let Some(command) = value.as_str() {
        if matches!(field, "expand_relationships" | "inspect_tests") && !looks_like_command(command)
        {
            return expansion_file_command(field, root, command);
        }
        return Some(command.to_string());
    }
    if value.as_bool() == Some(true) || value.as_u64() == Some(1) {
        let file = expansion_top_file(expansion, context)?;
        return expansion_file_command(field, root, &file);
    }
    if let Some(tool) = value.get("tool").and_then(serde_json::Value::as_str) {
        return expansion_tool_command(tool, value);
    }
    let target = expansion_focus_target(value, context)?;
    Some(focus_expansion_command(
        root,
        &target.file,
        target.symbol.as_deref(),
        target.line,
    ))
}

fn expansion_file_command(field: &str, root: &str, file: &str) -> Option<String> {
    let subcommand = match field {
        "expand_relationships" => "related",
        "inspect_tests" => "tests",
        _ => return None,
    };
    Some(format_callsieve_command(&[
        subcommand.to_string(),
        expansion_root_arg(root),
        "--file".to_string(),
        file.to_string(),
    ]))
}

fn expansion_tool_command(tool: &str, value: &serde_json::Value) -> Option<String> {
    let arguments = value.get("arguments").or_else(|| value.get("args"));
    let file = arguments
        .and_then(|arguments| arguments.get("file"))
        .and_then(serde_json::Value::as_str);
    let symbol = arguments
        .and_then(|arguments| arguments.get("symbol"))
        .and_then(serde_json::Value::as_str);
    let line = arguments
        .and_then(|arguments| arguments.get("line"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|line| usize::try_from(line).ok());
    let Some(file) = file else {
        return Some(tool.to_string());
    };
    let mut args = vec![tool.to_string(), "--file".to_string(), file.to_string()];
    if let Some(symbol) = symbol {
        args.push("--symbol".to_string());
        args.push(symbol.to_string());
    }
    if let Some(line) = line {
        args.push("--line".to_string());
        args.push(line.to_string());
    }
    Some(format_shell_command(&args))
}

fn expansion_top_file(
    expansion: &serde_json::Value,
    context: Option<&serde_json::Value>,
) -> Option<String> {
    expansion_command_value(expansion, "inspect_top_file")
        .and_then(|value| expansion_target_file(value, context))
}

fn expansion_target_file(
    value: &serde_json::Value,
    context: Option<&serde_json::Value>,
) -> Option<String> {
    if let Some(target) = expansion_focus_target(value, context) {
        return Some(target.file);
    }
    if let Some(file) = value
        .get("f")
        .or_else(|| value.get("file"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(file.to_string());
    }
    value
        .get("arguments")
        .or_else(|| value.get("args"))
        .and_then(|arguments| arguments.get("file"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn expansion_focus_target(
    value: &serde_json::Value,
    context: Option<&serde_json::Value>,
) -> Option<AgentFocusExpansion> {
    if let Some(index) = value.as_u64().and_then(|index| usize::try_from(index).ok()) {
        return expansion_focus_target_from_read_first_index(context?, index);
    }
    if let Some(items) = value.as_array() {
        let file = items.first()?.as_str()?.to_string();
        let symbol = items
            .get(1)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let line = items
            .get(2)
            .and_then(serde_json::Value::as_u64)
            .and_then(|line| usize::try_from(line).ok());
        return Some(AgentFocusExpansion { file, symbol, line });
    }

    let file = value
        .get("f")
        .or_else(|| value.get("file"))
        .and_then(serde_json::Value::as_str)?
        .to_string();
    let symbol = value
        .get("sy")
        .or_else(|| value.get("symbol"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let line = value
        .get("l")
        .or_else(|| value.get("line"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|line| usize::try_from(line).ok());
    Some(AgentFocusExpansion { file, symbol, line })
}

fn expansion_focus_target_from_read_first_index(
    context: &serde_json::Value,
    index: usize,
) -> Option<AgentFocusExpansion> {
    let target = context_value_read_first_targets(context)
        .into_iter()
        .nth(index)?;
    Some(agent_focus_expansion(&target))
}

fn focus_expansion_command(
    root: &str,
    file: &str,
    symbol: Option<&str>,
    line: Option<usize>,
) -> String {
    let mut args = vec![
        "focus".to_string(),
        expansion_root_arg(root),
        "--file".to_string(),
        file.to_string(),
    ];
    if let Some(symbol) = symbol {
        args.push("--symbol".to_string());
        args.push(symbol.to_string());
    }
    if let Some(line) = line {
        args.push("--line".to_string());
        args.push(line.to_string());
    }
    format_callsieve_command(&args)
}

fn expansion_root_arg(root: &str) -> String {
    if root.trim().is_empty() {
        ".".to_string()
    } else {
        root.to_string()
    }
}

fn looks_like_command(value: &str) -> bool {
    value.contains("callsieve ") || value.starts_with("callsieve_")
}

fn json_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_string)
}

fn json_string_any(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

fn json_usize(value: &serde_json::Value, path: &[&str]) -> Option<usize> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_u64().map(|number| number as usize)
}

fn json_usize_any(value: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
        .map(|number| number as usize)
}

fn json_array_any<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a Vec<serde_json::Value>> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_array))
}

fn symbol_name_from_value(symbol: &serde_json::Value) -> Option<&str> {
    symbol
        .get("n")
        .or_else(|| symbol.get("name"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            symbol
                .as_array()
                .and_then(|items| items.first())
                .and_then(serde_json::Value::as_str)
        })
}

fn symbol_kind_from_value(symbol: &serde_json::Value) -> Option<&str> {
    let kind = symbol
        .get("k")
        .or_else(|| symbol.get("kind"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            let items = symbol.as_array()?;
            items
                .get(3)
                .or_else(|| items.get(2).filter(|value| value.is_string()))
                .and_then(serde_json::Value::as_str)
        })?;
    Some(expand_compact_symbol_kind(kind))
}

fn expand_compact_symbol_kind(kind: &str) -> &str {
    match kind {
        "cl" => "class",
        "m" => "method",
        "if" => "interface",
        "t" => "type",
        "s" => "struct",
        "e" => "enum",
        "tr" => "trait",
        "im" => "impl",
        "c" => "constant",
        "mod" => "module",
        "mac" => "macro",
        "cmp" => "component",
        _ => kind,
    }
}

fn markdown_graph_hints(file: &serde_json::Value) -> Vec<String> {
    let Some(graph_hints) = file.get("g").or_else(|| file.get("graph_hints")) else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let upstream = json_string_array(graph_hints.get("u").or_else(|| graph_hints.get("upstream")));
    if !upstream.is_empty() {
        parts.push(format!("upstream {}", upstream.join(", ")));
    }
    let downstream = json_string_array(
        graph_hints
            .get("d")
            .or_else(|| graph_hints.get("downstream")),
    );
    if !downstream.is_empty() {
        parts.push(format!("downstream {}", downstream.join(", ")));
    }
    parts
}

fn markdown_call_paths(file: &serde_json::Value) -> Vec<String> {
    let Some(call_paths) = file.get("cp").or_else(|| file.get("call_paths")) else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let calls = json_call_path_array(
        call_paths.get("c").or_else(|| call_paths.get("calls")),
        "calls",
    );
    if !calls.is_empty() {
        parts.extend(calls);
    }
    let called_by = json_call_path_array(
        call_paths.get("by").or_else(|| call_paths.get("called_by")),
        "called_by",
    );
    if !called_by.is_empty() {
        parts.extend(called_by);
    }
    parts
}

fn json_call_path_array(value: Option<&serde_json::Value>, label: &str) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|edge| {
            let file = edge.get("f").or_else(|| edge.get("file"))?.as_str()?;
            let to = edge.get("t").or_else(|| edge.get("to"))?.as_str()?;
            let from = edge
                .get("fr")
                .or_else(|| edge.get("from"))
                .and_then(serde_json::Value::as_str);
            let line = edge
                .get("l")
                .or_else(|| edge.get("line"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let relation = if let Some(from) = from {
                format!("{from}->{to}")
            } else {
                to.to_string()
            };
            Some(format!("{label} {relation} in {file}:{line}"))
        })
        .collect()
}

fn mcp_config_output(root: &Path, format: McpConfigFormat) -> McpConfigOutput {
    let callsieve_command = callsieve_executable_display();
    let instructions = vec![
        "Register this config with any AI CLI that supports MCP stdio servers.".to_string(),
        format!(
            "First task command remains: callsieve agent-context {} \"<task>\"",
            root.display()
        ),
        "Use callsieve_context before broad grep, repository search, or repeated file reads."
            .to_string(),
    ];

    match format {
        McpConfigFormat::Json => McpConfigOutput {
            command: "mcp-config",
            root: root_label(root),
            format: "json",
            instructions,
            config: Some(mcp_config_json(&callsieve_command)),
            config_text: None,
        },
        McpConfigFormat::Toml => McpConfigOutput {
            command: "mcp-config",
            root: root_label(root),
            format: "toml",
            instructions,
            config: None,
            config_text: Some(mcp_config_toml(&callsieve_command)),
        },
    }
}

fn mcp_config_json(callsieve_command: &str) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            "callsieve": {
                "type": "stdio",
                "command": callsieve_command,
                "args": ["mcp"],
                "env": {}
            }
        }
    })
}

fn mcp_config_toml(callsieve_command: &str) -> String {
    format!(
        "[mcp_servers.callsieve]\ncommand = {}\nargs = [\"mcp\"]\nstartup_timeout_sec = 20\ntool_timeout_sec = 60\n",
        toml_basic_string(callsieve_command)
    )
}

fn mcp_registry_manifest() -> serde_json::Value {
    serde_json::json!({
        "$schema": "https://static.modelcontextprotocol.io/schemas/2025-12-11/server.schema.json",
        "name": "io.github.philipjohnbasile/callsieve",
        "title": "CallSieve",
        "description": "Local-first codebase retrieval for AI coding agents. Runs `callsieve mcp` over stdio and indexes repositories locally without cloud services or API keys.",
        "version": env!("CARGO_PKG_VERSION"),
        "packages": [
            {
                "registryType": "oci",
                "identifier": format!("ghcr.io/philipjohnbasile/callsieve:{}", env!("CARGO_PKG_VERSION")),
                "version": env!("CARGO_PKG_VERSION"),
                "transport": {
                    "type": "stdio",
                    "args": ["mcp"]
                }
            }
        ],
        "_meta": {
            "io.modelcontextprotocol.registry/publisher-provided": {
                "local_first": true,
                "generated_by": "callsieve mcp-registry-manifest",
                "server_command": "callsieve mcp",
                "publishing": "descriptor only; this command does not contact the network or publish"
            }
        }
    })
}

fn mcp_contract_output() -> McpContractOutput {
    McpContractOutput {
        command: "mcp-contract",
        contract: public_mcp_contract_evidence(),
    }
}

fn agent_native_protocol_output() -> AgentNativeProtocolOutput {
    AgentNativeProtocolOutput {
        command: "agent-native-protocol",
        schema_version: 1,
        status: "pass",
        claim_boundary: "Protocol only: this artifact defines how to collect approved agent-native search measurements. It does not claim a measured win until transcript/export-backed baselines are added to public proof.",
        workflow: vec![
            AgentNativeProtocolStep {
                step: 1,
                name: "prepare_callsieve_side",
                command: "callsieve agent-native-template <repo> <tasks-or-public-manifest.json> --k 5 --out <task-log.json>",
                evidence: "unmeasured task log with expected_files, callsieve_files, callsieve_packet_tokens, source task hash, index fingerprint, and retrieval contract fingerprint",
            },
            AgentNativeProtocolStep {
                step: 2,
                name: "record_native_search_run",
                command: "run the approved native agent on the same pinned tasks without using CallSieve for the native side",
                evidence: "transcript or native-search export showing files selected/read and transcript-backed context-token counts",
            },
            AgentNativeProtocolStep {
                step: 3,
                name: "preflight_measurement",
                command: "callsieve agent-native-check <filled-task-log.json> --mode measured --source-artifact <transcript-or-export>...",
                evidence: "status pass with measured agent_native_files, agent_native_context_tokens, recording_status=measured, and hashed source artifacts",
            },
            AgentNativeProtocolStep {
                step: 4,
                name: "standardize_baseline",
                command: "callsieve agent-native-baseline <filled-task-log.json> --source-artifact <transcript-or-export>... --out <baseline.json>",
                evidence: "locally measured baseline artifact with first-correct rates, context-token ratios, source_artifact_evidence, and transcript_provenance_status pass",
            },
            AgentNativeProtocolStep {
                step: 5,
                name: "publish_guardrail",
                command: "add <baseline.json> to public-proof manifest agent_native_search_baselines and rerun callsieve public-proof-report <manifest.json>",
                evidence: "public proof passes only after source artifacts are reread and byte counts and hashes are recomputed",
            },
        ],
        required_task_fields: AgentNativeProtocolTaskFields {
            template: vec![
                "id",
                "task",
                "expected_files",
                "callsieve_files",
                "callsieve_packet_tokens",
                "recording_status=needs_agent_native_measurement",
                "agent_native_files=[]",
                "agent_native_context_tokens=0",
            ],
            measured: vec![
                "id",
                "expected_files",
                "callsieve_files",
                "callsieve_packet_tokens",
                "agent_native_files",
                "agent_native_context_tokens",
                "recording_status=measured",
            ],
            recording_status: vec![
                "needs_agent_native_measurement before the external run",
                "measured after agent_native_files and transcript-backed agent_native_context_tokens are filled",
            ],
        },
        source_artifact_requirements: AgentNativeProtocolSourceArtifacts {
            required_roles: vec!["task_log", "agent_native_transcript_or_export"],
            hash_algorithm: "fnv1a64 stable_content_hash",
            requirements: vec![
                "at least one transcript or native-search export is required for measured baselines",
                "source artifacts must be readable locally when public proof runs",
                "public proof recomputes source artifact byte counts and hashes",
                "artifacts must not require network access, cloud credentials, or proprietary upload",
            ],
        },
        validation_commands: vec![
            "callsieve agent-native-check <task-log.json> --mode template",
            "callsieve agent-native-check <filled-task-log.json> --mode measured --source-artifact <transcript-or-export>...",
            "callsieve agent-native-baseline <filled-task-log.json> --source-artifact <transcript-or-export>... --out <baseline.json>",
            "callsieve public-proof-report <public-proof-manifest.json>",
        ],
        public_proof_integration: vec![
            "terminal_artifacts id=agent-native-template validates the unmeasured template by recomputing CallSieve files and packet tokens from current source/index/retrieval provenance",
            "terminal_artifacts id=agent-native-protocol validates this protocol artifact against the live CLI output",
            "terminal_artifacts id=agent-native-check validates the measured preflight output by recomputing it from the task log and transcript/export source artifacts, and one passing check artifact must contain all source hashes for each measured baseline before it can support public proof",
            "agent_native_search_baselines entries are optional until real approved external-agent runs exist",
            "required agent_native_search_baselines fail closed when missing, not locally measured, below thresholds, lacking recomputed transcript/export provenance, missing the checked measured preflight artifact, or not hash-linked to that preflight artifact",
        ],
    }
}

fn retrieval_manifest_root(value: &serde_json::Value) -> PathBuf {
    let raw_path = ["path", "repo", "root"]
        .iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str));
    let Some(raw_path) = raw_path else {
        return PathBuf::from(".");
    };

    PathBuf::from(raw_path)
}

struct ProofRehearsalOptions {
    preflight: bool,
    fix: bool,
    resume: bool,
    collect_ollama: bool,
    ollama_manifest: PathBuf,
    ollama_model: String,
    ollama_limit: usize,
    ollama_context_limit: usize,
    retry_count: usize,
    ledger: PathBuf,
}

fn proof_rehearsal(options: ProofRehearsalOptions) -> Result<ProofRehearsalOutput> {
    let fixes = if options.fix {
        proof_rehearsal_safe_fixes(options.retry_count)?
    } else {
        Vec::new()
    };
    let preflight = proof_rehearsal_preflight();
    let command_matrix = proof_rehearsal_command_matrix();
    let next_observed_gate = format!("callsieve pilot-qa {}", OBSERVED_CODEX_OSS_50_MANIFEST);

    if options.preflight || preflight.status != "pass" {
        return Ok(ProofRehearsalOutput {
            command: "proof-rehearsal",
            status: preflight.status.clone(),
            mode: "preflight",
            ledger: options.ledger.display().to_string(),
            command_matrix,
            preflight,
            context_payload_reduction: None,
            fixes,
            steps: Vec::new(),
            claim_proof_included: false,
            next_observed_gate,
        });
    }

    let previous_ledger = if options.resume && options.ledger.is_file() {
        read_rehearsal_ledger(&options.ledger).ok()
    } else {
        None
    };
    let started_at = now_unix_seconds();
    let mut ledger = ProofRehearsalLedger {
        schema_version: 1,
        command_matrix: "proof-rehearsal-v2-rust".to_string(),
        status: "running".to_string(),
        started_at,
        updated_at: started_at,
        finished_at: None,
        error: None,
        fixes: fixes.clone(),
        steps: Vec::new(),
    };
    write_rehearsal_ledger(&options.ledger, &ledger)?;

    run_proof_rehearsal_step(
        &options.ledger,
        &mut ledger,
        previous_ledger.as_ref(),
        options.resume,
        "retrieval",
        "retrieval fixtures",
        &format!("callsieve eval-retrieval {REHEARSAL_RETRIEVAL_FIXTURES}"),
        options.retry_count,
        || {
            let output = eval_retrieval_file(
                Path::new(REHEARSAL_RETRIEVAL_FIXTURES),
                8,
                REHEARSAL_SNIPPETS_PER_FILE,
                true,
            )?;
            let value = serde_json::to_value(&output)?;
            assert_retrieval_rehearsal(&value)?;
            Ok(step_summary(&value))
        },
    )?;

    run_proof_rehearsal_step(
        &options.ledger,
        &mut ledger,
        previous_ledger.as_ref(),
        options.resume,
        "perf",
        "local perf report",
        "callsieve perf-report . --iterations 5",
        options.retry_count,
        || {
            let output = perf_report(Path::new("."), None, 5)?;
            let value = serde_json::to_value(&output)?;
            Ok(step_summary(&value))
        },
    )?;

    run_proof_rehearsal_step(
        &options.ledger,
        &mut ledger,
        previous_ledger.as_ref(),
        options.resume,
        "doctor",
        "external benchmark doctor",
        &format!("callsieve benchmark-doctor {REHEARSAL_EXTERNAL_MANIFEST}"),
        options.retry_count,
        || {
            let output = benchmark_doctor_file(Path::new(REHEARSAL_EXTERNAL_MANIFEST))?;
            let value = serde_json::to_value(&output)?;
            assert_status_pass(&value, "benchmark-doctor")?;
            Ok(serde_json::json!({
                "status": value.get("status").cloned().unwrap_or_default(),
                "repos": value.get("repos").cloned().unwrap_or_default(),
                "checks": value.get("checks").cloned().unwrap_or_default(),
                "failures": value.get("failures").cloned().unwrap_or_default()
            }))
        },
    )?;

    run_proof_rehearsal_step(
        &options.ledger,
        &mut ledger,
        previous_ledger.as_ref(),
        options.resume,
        "benchmark_initial",
        "external benchmark report before replay regeneration",
        &format!(
            "callsieve benchmark-report {REHEARSAL_EXTERNAL_MANIFEST} --limit {REHEARSAL_REPORT_LIMIT}"
        ),
        options.retry_count,
        || {
            let output = benchmark_report_file(
                Path::new(REHEARSAL_EXTERNAL_MANIFEST),
                REHEARSAL_REPORT_LIMIT,
                REHEARSAL_SNIPPETS_PER_FILE,
                true,
            )?;
            let value = serde_json::to_value(&output)?;
            assert_external_report_complete(&value)?;
            Ok(step_summary(&value))
        },
    )?;

    for fixture in EXTERNAL_BENCHMARK_FIXTURES {
        let id = format!(
            "trace_{}",
            Path::new(fixture.repo)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(fixture.repo)
                .replace('-', "_")
        );
        let description = format!("controlled replay {}", fixture.repo);
        let signature = format!(
            "callsieve trace-replay {} {} {} --limit {}",
            fixture.repo, fixture.suite, fixture.trace, REHEARSAL_REPORT_LIMIT
        );
        run_proof_rehearsal_step(
            &options.ledger,
            &mut ledger,
            previous_ledger.as_ref(),
            options.resume,
            &id,
            &description,
            &signature,
            options.retry_count,
            || {
                let replay = trace_replay_file(
                    Path::new(fixture.repo),
                    Path::new(fixture.suite),
                    Path::new(fixture.trace),
                    REHEARSAL_REPORT_LIMIT,
                    REHEARSAL_SNIPPETS_PER_FILE,
                    true,
                )?;
                let value = serde_json::to_value(&replay)?;
                Ok(serde_json::json!({
                    "trace": fixture.trace,
                    "repo": fixture.repo,
                    "suite": fixture.suite,
                    "collection": "controlled_replay",
                    "tasks": value
                        .get("tasks")
                        .and_then(serde_json::Value::as_array)
                        .map(Vec::len)
                        .unwrap_or_default()
                }))
            },
        )?;
    }

    run_proof_rehearsal_step(
        &options.ledger,
        &mut ledger,
        previous_ledger.as_ref(),
        options.resume,
        "benchmark_final",
        "external benchmark report after replay regeneration",
        &format!(
            "callsieve benchmark-report {REHEARSAL_EXTERNAL_MANIFEST} --limit {REHEARSAL_REPORT_LIMIT}"
        ),
        options.retry_count,
        || {
            let output = benchmark_report_file(
                Path::new(REHEARSAL_EXTERNAL_MANIFEST),
                REHEARSAL_REPORT_LIMIT,
                REHEARSAL_SNIPPETS_PER_FILE,
                true,
            )?;
            let value = serde_json::to_value(&output)?;
            assert_external_report_complete(&value)?;
            Ok(step_summary(&value))
        },
    )?;

    if options.collect_ollama {
        let signature = format!(
            "callsieve pilot-collect-ollama {} --model {} --limit {} --context-limit {}",
            options.ollama_manifest.display(),
            options.ollama_model,
            options.ollama_limit,
            options.ollama_context_limit
        );
        let ollama_manifest = options.ollama_manifest.clone();
        let ollama_model = options.ollama_model.clone();
        run_proof_rehearsal_step(
            &options.ledger,
            &mut ledger,
            previous_ledger.as_ref(),
            options.resume,
            "ollama_supplemental",
            "supplemental Ollama collection",
            &signature,
            options.retry_count,
            || {
                let output = pilot_collect_ollama(
                    &ollama_manifest,
                    &ollama_model,
                    options.ollama_limit,
                    options.ollama_context_limit,
                    REHEARSAL_SNIPPETS_PER_FILE,
                    48,
                    240,
                )?;
                let value = serde_json::to_value(&output)?;
                Ok(step_summary(&value))
            },
        )?;
    }

    ledger.status = "pass".to_string();
    ledger.updated_at = now_unix_seconds();
    ledger.finished_at = Some(ledger.updated_at);
    write_rehearsal_ledger(&options.ledger, &ledger)?;
    let context_payload_reduction = proof_rehearsal_context_payload_reduction()?;

    Ok(ProofRehearsalOutput {
        command: "proof-rehearsal",
        status: ledger.status.clone(),
        mode: "rehearsal",
        ledger: options.ledger.display().to_string(),
        command_matrix,
        preflight,
        context_payload_reduction: Some(context_payload_reduction),
        fixes,
        steps: ledger.steps,
        claim_proof_included: false,
        next_observed_gate,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_proof_rehearsal_step<F>(
    ledger_path: &Path,
    ledger: &mut ProofRehearsalLedger,
    previous_ledger: Option<&ProofRehearsalLedger>,
    resume: bool,
    id: &str,
    description: &str,
    signature: &str,
    retry_count: usize,
    mut run: F,
) -> Result<serde_json::Value>
where
    F: FnMut() -> Result<serde_json::Value>,
{
    if resume && let Some(previous) = previous_rehearsal_step(previous_ledger, id, signature) {
        let now = now_unix_seconds();
        let record = ProofRehearsalStepRecord {
            id: id.to_string(),
            description: description.to_string(),
            command: signature.to_string(),
            signature: signature.to_string(),
            status: "pass".to_string(),
            skipped: true,
            attempts: 0,
            started_at: now,
            finished_at: now,
            summary: previous.summary.clone(),
            error: None,
        };
        ledger.steps.push(record);
        ledger.updated_at = now;
        write_rehearsal_ledger(ledger_path, ledger)?;
        return Ok(previous.summary.clone());
    }

    let started_at = now_unix_seconds();
    let attempts = retry_count.saturating_add(1).max(1);
    let mut last_error = String::new();
    for attempt in 1..=attempts {
        match run() {
            Ok(summary) => {
                let finished_at = now_unix_seconds();
                let record = ProofRehearsalStepRecord {
                    id: id.to_string(),
                    description: description.to_string(),
                    command: signature.to_string(),
                    signature: signature.to_string(),
                    status: "pass".to_string(),
                    skipped: false,
                    attempts: attempt,
                    started_at,
                    finished_at,
                    summary: summary.clone(),
                    error: None,
                };
                ledger.steps.push(record);
                ledger.updated_at = finished_at;
                write_rehearsal_ledger(ledger_path, ledger)?;
                return Ok(summary);
            }
            Err(error) => {
                last_error = error.to_string();
            }
        }
    }

    let finished_at = now_unix_seconds();
    let record = ProofRehearsalStepRecord {
        id: id.to_string(),
        description: description.to_string(),
        command: signature.to_string(),
        signature: signature.to_string(),
        status: "fail".to_string(),
        skipped: false,
        attempts,
        started_at,
        finished_at,
        summary: serde_json::Value::Null,
        error: Some(last_error.clone()),
    };
    ledger.steps.push(record);
    ledger.status = "fail".to_string();
    ledger.error = Some(last_error.clone());
    ledger.updated_at = finished_at;
    ledger.finished_at = Some(finished_at);
    write_rehearsal_ledger(ledger_path, ledger)?;
    anyhow::bail!("proof rehearsal step failed: {description}: {last_error}")
}

fn previous_rehearsal_step<'a>(
    previous_ledger: Option<&'a ProofRehearsalLedger>,
    id: &str,
    signature: &str,
) -> Option<&'a ProofRehearsalStepRecord> {
    previous_ledger?
        .steps
        .iter()
        .rev()
        .find(|step| step.id == id && step.signature == signature && step.status == "pass")
}

fn read_rehearsal_ledger(path: &Path) -> Result<ProofRehearsalLedger> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("failed to read rehearsal ledger: {}", path.display()))?;
    serde_json::from_str(&json)
        .with_context(|| format!("failed to parse rehearsal ledger: {}", path.display()))
}

fn write_rehearsal_ledger(path: &Path, ledger: &ProofRehearsalLedger) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(ledger)?)
        .with_context(|| format!("failed to write rehearsal ledger: {}", path.display()))
}

fn proof_rehearsal_safe_fixes(retry_count: usize) -> Result<Vec<ProofRehearsalFix>> {
    let mut fixes = Vec::new();
    ensure_rehearsal_dir(Path::new("benchmarks/evidence"), &mut fixes)?;
    for fixture in EXTERNAL_BENCHMARK_FIXTURES {
        if let Some(parent) = Path::new(fixture.trace)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            ensure_rehearsal_dir(parent, &mut fixes)?;
        }
    }

    retry_rehearsal_fix("index root", retry_count, || {
        build_index_output(Path::new("."), false)
    })?;
    fixes.push(ProofRehearsalFix {
        fix: "index".to_string(),
        path: ".".to_string(),
        status: "applied".to_string(),
    });

    for fixture in EXTERNAL_BENCHMARK_FIXTURES {
        let repo = Path::new(fixture.repo);
        if repo.is_dir() {
            retry_rehearsal_fix(&format!("index {}", fixture.repo), retry_count, || {
                build_index_output(repo, false)
            })?;
            fixes.push(ProofRehearsalFix {
                fix: "index".to_string(),
                path: fixture.repo.to_string(),
                status: "applied".to_string(),
            });
        } else {
            fixes.push(ProofRehearsalFix {
                fix: "index".to_string(),
                path: fixture.repo.to_string(),
                status: "skipped_missing_repo".to_string(),
            });
        }

        let suite = Path::new(fixture.suite);
        let trace = Path::new(fixture.trace);
        if repo.is_dir() && suite.is_file() && !trace.is_file() {
            retry_rehearsal_fix(&format!("trace {}", fixture.trace), retry_count, || {
                trace_replay_file(
                    repo,
                    suite,
                    trace,
                    REHEARSAL_REPORT_LIMIT,
                    REHEARSAL_SNIPPETS_PER_FILE,
                    true,
                )
            })?;
            fixes.push(ProofRehearsalFix {
                fix: "trace-replay".to_string(),
                path: fixture.trace.to_string(),
                status: "applied".to_string(),
            });
        }
    }

    Ok(fixes)
}

fn retry_rehearsal_fix<T, F>(description: &str, retry_count: usize, mut run: F) -> Result<T>
where
    F: FnMut() -> Result<T>,
{
    let attempts = retry_count.saturating_add(1).max(1);
    let mut last_error = String::new();
    for _ in 0..attempts {
        match run() {
            Ok(output) => return Ok(output),
            Err(error) => last_error = error.to_string(),
        }
    }
    anyhow::bail!("safe fix failed: {description}: {last_error}")
}

fn ensure_rehearsal_dir(path: &Path, fixes: &mut Vec<ProofRehearsalFix>) -> Result<()> {
    if !path.is_dir() {
        fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;
        fixes.push(ProofRehearsalFix {
            fix: "create_directory".to_string(),
            path: path.display().to_string(),
            status: "applied".to_string(),
        });
    }
    Ok(())
}

fn proof_rehearsal_preflight() -> ProofRehearsalPreflightOutput {
    let mut checks = Vec::new();
    push_rehearsal_check(
        &mut checks,
        "retrieval_fixtures",
        REHEARSAL_RETRIEVAL_FIXTURES,
        Path::new(REHEARSAL_RETRIEVAL_FIXTURES).is_file(),
        "retrieval fixture manifest exists",
        "retrieval fixture manifest is missing",
    );
    push_rehearsal_check(
        &mut checks,
        "external_manifest",
        REHEARSAL_EXTERNAL_MANIFEST,
        Path::new(REHEARSAL_EXTERNAL_MANIFEST).is_file(),
        "external benchmark manifest exists",
        "external benchmark manifest is missing",
    );
    push_rehearsal_check(
        &mut checks,
        "root_index",
        ".callsieve/index.json",
        store::json_store::index_path(Path::new(".")).is_file(),
        "root index exists",
        "root index is missing",
    );

    for fixture in EXTERNAL_BENCHMARK_FIXTURES {
        push_rehearsal_check(
            &mut checks,
            "fixture_repo",
            fixture.repo,
            Path::new(fixture.repo).is_dir(),
            "fixture repo exists",
            "fixture repo is missing",
        );
        push_rehearsal_check(
            &mut checks,
            "fixture_index",
            &store::json_store::index_path(Path::new(fixture.repo))
                .display()
                .to_string(),
            store::json_store::index_path(Path::new(fixture.repo)).is_file(),
            "fixture index exists",
            "fixture index is missing",
        );
        push_rehearsal_check(
            &mut checks,
            "fixture_suite",
            fixture.suite,
            Path::new(fixture.suite).is_file(),
            "fixture suite exists",
            "fixture suite is missing",
        );
        push_rehearsal_check(
            &mut checks,
            "controlled_trace",
            fixture.trace,
            Path::new(fixture.trace).is_file(),
            "controlled replay trace exists",
            "controlled replay trace is missing",
        );
    }

    let failures = checks.iter().filter(|check| check.status == "fail").count();
    ProofRehearsalPreflightOutput {
        status: if failures == 0 { "pass" } else { "fail" }.to_string(),
        failures,
        checks,
    }
}

fn push_rehearsal_check(
    checks: &mut Vec<ProofRehearsalCheck>,
    check: &str,
    path: &str,
    passed: bool,
    pass_message: &str,
    fail_message: &str,
) {
    checks.push(ProofRehearsalCheck {
        check: check.to_string(),
        path: path.to_string(),
        status: if passed { "pass" } else { "fail" }.to_string(),
        message: if passed {
            pass_message.to_string()
        } else {
            fail_message.to_string()
        },
    });
}

fn proof_rehearsal_command_matrix() -> ProofRehearsalCommandMatrix {
    ProofRehearsalCommandMatrix {
        retrieval_fixtures: REHEARSAL_RETRIEVAL_FIXTURES.to_string(),
        external_manifest: REHEARSAL_EXTERNAL_MANIFEST.to_string(),
        external_fixtures: EXTERNAL_BENCHMARK_FIXTURES.to_vec(),
        report_limit: REHEARSAL_REPORT_LIMIT,
        perf_iterations: 5,
        context_payload_reduction_included: true,
        context_payload_scope: "agent_platform_neutral",
        includes_proof_report: false,
    }
}

fn eval_retrieval_file(
    manifest: &Path,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<query::EvalRetrievalOutput> {
    let manifest_json = fs::read_to_string(manifest).with_context(|| {
        format!(
            "failed to read retrieval eval manifest: {}",
            manifest.display()
        )
    })?;
    let manifest_value: serde_json::Value =
        serde_json::from_str(&manifest_json).with_context(|| {
            format!(
                "failed to parse retrieval eval manifest: {}",
                manifest.display()
            )
        })?;
    let root = retrieval_manifest_root(&manifest_value);
    let index = store::json_store::load_index(&root)?;
    let suite: query::BenchmarkSuiteInput =
        serde_json::from_value(manifest_value).with_context(|| {
            format!(
                "failed to parse retrieval eval tasks: {}",
                manifest.display()
            )
        })?;
    query::eval_retrieval(
        &root,
        &index,
        suite,
        limit,
        snippets_per_file,
        include_snippets,
    )
}

fn benchmark_doctor_file(manifest: &Path) -> Result<query::BenchmarkDoctorOutput> {
    let manifest_json = fs::read_to_string(manifest)
        .with_context(|| format!("failed to read benchmark manifest: {}", manifest.display()))?;
    query::benchmark_doctor_from_str(&manifest_json)
        .with_context(|| format!("failed to check benchmark manifest: {}", manifest.display()))
}

fn benchmark_report_file(
    manifest: &Path,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<query::BenchmarkReportOutput> {
    let manifest_json = fs::read_to_string(manifest)
        .with_context(|| format!("failed to read benchmark manifest: {}", manifest.display()))?;
    let manifest: query::BenchmarkReportManifest = serde_json::from_str(&manifest_json)
        .with_context(|| format!("failed to parse benchmark manifest: {}", manifest.display()))?;
    query::benchmark_report(manifest, limit, snippets_per_file, include_snippets)
}

fn competitive_report_file(
    manifest_path: &Path,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<CompetitiveReportOutput> {
    let manifest_json = fs::read_to_string(manifest_path).with_context(|| {
        format!(
            "failed to read competitive report manifest: {}",
            manifest_path.display()
        )
    })?;
    let manifest: CompetitiveReportManifest =
        serde_json::from_str(&manifest_json).with_context(|| {
            format!(
                "failed to parse competitive report manifest: {}",
                manifest_path.display()
            )
        })?;

    let trace_policy = query::benchmark_report_trace_policy_check(&manifest.benchmark)?;
    let trace_policy_value = serde_json::to_value(&trace_policy)?;
    let benchmark = query::benchmark_report(
        manifest.benchmark,
        limit,
        snippets_per_file,
        include_snippets,
    )?;
    let benchmark_value = serde_json::to_value(&benchmark)?;
    let mut callsieve = competitive_callsieve_evidence(&benchmark_value, Some(&trace_policy_value));

    let (natural_language, natural_language_benchmark) =
        if let Some(natural_language_manifest) = manifest.natural_language_benchmark {
            let output = query::benchmark_report(
                natural_language_manifest,
                limit,
                snippets_per_file,
                include_snippets,
            )?;
            let value = serde_json::to_value(&output)?;
            (
                Some(competitive_callsieve_evidence(&value, None)),
                Some(value),
            )
        } else {
            (None, None)
        };

    let performance = if let Some(performance_manifest) = manifest.performance {
        let output = perf_report(
            &performance_manifest.path,
            performance_manifest.tasks.as_deref(),
            performance_manifest.iterations,
        )?;
        callsieve.first_query_p50_ms = Some(output.summary.p50_ms);
        callsieve.first_query_p95_ms = Some(output.summary.p95_ms);
        callsieve.first_query_samples = Some(output.summary.samples);
        Some(serde_json::to_value(&output)?)
    } else {
        None
    };

    let missing_required_agents =
        missing_required_agents(&callsieve.agent_coverage, &manifest.targets.required_agents);
    callsieve.missing_required_agents = missing_required_agents.clone();

    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    if callsieve.first_correct_file_rate_at_k
        < manifest.targets.minimum_first_correct_file_rate_at_k
    {
        failures.push(format!(
            "first_correct_file_rate_at_k {:.3} is below target {:.3}",
            callsieve.first_correct_file_rate_at_k,
            manifest.targets.minimum_first_correct_file_rate_at_k
        ));
    }
    if callsieve.expected_file_recall < manifest.targets.minimum_expected_file_recall {
        failures.push(format!(
            "expected_file_recall {:.3} is below target {:.3}",
            callsieve.expected_file_recall, manifest.targets.minimum_expected_file_recall
        ));
    }
    match &natural_language {
        Some(natural_language) => {
            if natural_language.first_correct_file_rate_at_k
                < manifest
                    .targets
                    .minimum_natural_language_first_correct_file_rate_at_k
            {
                failures.push(format!(
                    "natural_language_first_correct_file_rate_at_k {:.3} is below target {:.3}",
                    natural_language.first_correct_file_rate_at_k,
                    manifest
                        .targets
                        .minimum_natural_language_first_correct_file_rate_at_k
                ));
            }
            if natural_language.expected_file_recall
                < manifest
                    .targets
                    .minimum_natural_language_expected_file_recall
            {
                failures.push(format!(
                    "natural_language_expected_file_recall {:.3} is below target {:.3}",
                    natural_language.expected_file_recall,
                    manifest
                        .targets
                        .minimum_natural_language_expected_file_recall
                ));
            }
        }
        None if manifest.targets.require_natural_language_benchmark => {
            failures.push("natural-language benchmark is required but missing".to_string());
        }
        None => warnings.push(
            "natural-language benchmark not provided; natural-language recall is unmeasured"
                .to_string(),
        ),
    }
    if manifest.targets.require_trace_or_receipt_proof
        && !callsieve.trace_or_receipt_proof_available
    {
        failures.push("trace or receipt proof is required but missing".to_string());
    }
    if let Some(max_packet_tokens) = manifest.targets.maximum_default_packet_tokens
        && callsieve.default_packet_tokens > max_packet_tokens
    {
        failures.push(format!(
            "default_packet_tokens {} exceeds target {}",
            callsieve.default_packet_tokens, max_packet_tokens
        ));
    }
    if manifest.targets.require_zero_retrieval_model_tokens && callsieve.retrieval_model_tokens != 0
    {
        failures.push(format!(
            "retrieval_model_tokens {} exceeds required zero-token retrieval target",
            callsieve.retrieval_model_tokens
        ));
    }
    if manifest.targets.require_zero_per_query_retrieval_cost
        && callsieve.per_query_retrieval_cost_usd > f64::EPSILON
    {
        failures.push(format!(
            "per_query_retrieval_cost_usd {:.6} exceeds required zero-cost retrieval target",
            callsieve.per_query_retrieval_cost_usd
        ));
    }
    if manifest.targets.require_no_default_code_upload && callsieve.default_code_upload_required {
        failures.push("default_code_upload_required must be false".to_string());
    }
    if callsieve.agent_coverage_count < manifest.targets.minimum_agent_coverage {
        failures.push(format!(
            "agent_coverage_count {} is below target {}",
            callsieve.agent_coverage_count, manifest.targets.minimum_agent_coverage
        ));
    }
    if !missing_required_agents.is_empty() {
        failures.push(format!(
            "required agent coverage is missing: {}",
            missing_required_agents.join(", ")
        ));
    }
    if let Some(max_p95_ms) = manifest.targets.maximum_first_query_p95_ms {
        match callsieve.first_query_p95_ms {
            Some(actual) if actual <= max_p95_ms => {}
            Some(actual) => failures.push(format!(
                "first_query_p95_ms {actual} exceeds target {max_p95_ms}"
            )),
            None => failures.push(
                "maximum_first_query_p95_ms target is set but performance benchmark is missing"
                    .to_string(),
            ),
        }
    } else if callsieve.first_query_p95_ms.is_none() {
        warnings
            .push("performance benchmark not provided; first-query time is unmeasured".to_string());
    }
    if manifest.competitors.is_empty() {
        warnings.push("no competitor rows declared in manifest".to_string());
    }

    let mut comparison_matrix = Vec::new();
    comparison_matrix.push(callsieve_comparison_row(
        &callsieve,
        natural_language.as_ref(),
    ));
    comparison_matrix.extend(
        manifest
            .competitors
            .into_iter()
            .map(competitor_comparison_row),
    );

    let mut next_actions = Vec::new();
    match &natural_language {
        None => next_actions.push(
            "add a natural-language benchmark manifest when making natural-language recall claims"
                .to_string(),
        ),
        Some(natural_language)
            if natural_language.first_correct_file_rate_at_k
                < manifest
                    .targets
                    .minimum_natural_language_first_correct_file_rate_at_k =>
        {
            next_actions.push(
                "improve deterministic natural-language retrieval before raising NL claims"
                    .to_string(),
            );
        }
        Some(_) => {}
    }
    if callsieve.first_query_p95_ms.is_some() {
        next_actions.push(
            "keep first-query latency in the competitive manifest so speed claims stay measured"
                .to_string(),
        );
    } else {
        next_actions.push("add performance evidence before making speed claims".to_string());
    }
    if !callsieve.trace_or_receipt_proof_available {
        next_actions.push(
            "include observed trace or receipt evidence before claiming grep/read-loop avoidance"
                .to_string(),
        );
    }
    if callsieve.agent_coverage_count < manifest.targets.minimum_agent_coverage
        || !missing_required_agents.is_empty()
    {
        next_actions.push(
            "keep agent setup coverage broad enough to defend the agent-neutrality claim"
                .to_string(),
        );
    }
    if callsieve.expected_file_recall < manifest.targets.minimum_expected_file_recall {
        next_actions.push(
            "improve expected-file recall before raising broad context-quality claims".to_string(),
        );
    }
    if manifest
        .targets
        .maximum_default_packet_tokens
        .is_some_and(|target| callsieve.default_packet_tokens > target)
    {
        next_actions
            .push("keep the default packet compact enough to beat broad repo packing".to_string());
    }
    next_actions.push(
        "update competitor rows only from cited public sources or local measurements".to_string(),
    );

    Ok(CompetitiveReportOutput {
        command: "competitive-report",
        status: if failures.is_empty() {
            "pass".to_string()
        } else {
            "needs_work".to_string()
        },
        generated_at: now_unix_seconds(),
        goal: manifest.goal,
        targets: CompetitiveReportTargetsOutput {
            minimum_first_correct_file_rate_at_k: manifest
                .targets
                .minimum_first_correct_file_rate_at_k,
            minimum_expected_file_recall: manifest.targets.minimum_expected_file_recall,
            minimum_natural_language_first_correct_file_rate_at_k: manifest
                .targets
                .minimum_natural_language_first_correct_file_rate_at_k,
            minimum_natural_language_expected_file_recall: manifest
                .targets
                .minimum_natural_language_expected_file_recall,
            maximum_first_query_p95_ms: manifest.targets.maximum_first_query_p95_ms,
            maximum_default_packet_tokens: manifest.targets.maximum_default_packet_tokens,
            require_natural_language_benchmark: manifest.targets.require_natural_language_benchmark,
            require_trace_or_receipt_proof: manifest.targets.require_trace_or_receipt_proof,
            require_zero_retrieval_model_tokens: manifest
                .targets
                .require_zero_retrieval_model_tokens,
            require_zero_per_query_retrieval_cost: manifest
                .targets
                .require_zero_per_query_retrieval_cost,
            require_no_default_code_upload: manifest.targets.require_no_default_code_upload,
            minimum_agent_coverage: manifest.targets.minimum_agent_coverage,
            required_agents: manifest.targets.required_agents,
        },
        callsieve,
        natural_language,
        comparison_matrix,
        benchmark: benchmark_value,
        natural_language_benchmark,
        performance,
        trace_policy: trace_policy_value,
        failures,
        warnings,
        next_actions,
    })
}

fn public_proof_report_file(
    manifest_path: &Path,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<PublicProofReportOutput> {
    let manifest_json = fs::read_to_string(manifest_path).with_context(|| {
        format!(
            "failed to read public proof manifest: {}",
            manifest_path.display()
        )
    })?;
    let manifest: PublicProofManifest =
        serde_json::from_str(&manifest_json).with_context(|| {
            format!(
                "failed to parse public proof manifest: {}",
                manifest_path.display()
            )
        })?;
    let targets = manifest.targets.clone();
    let competitive = competitive_report_file(
        &manifest.competitive_report,
        limit,
        snippets_per_file,
        include_snippets,
    )?;
    let local_gate = public_proof_local_gate(&manifest.competitive_report, &competitive);
    let agent_setup = public_agent_setup_evidence(&competitive);
    let broad_read_guardrail = public_broad_read_guardrail(&competitive.callsieve, &targets);
    let context_packet_guardrail = public_context_packet_guardrail(&competitive.callsieve);

    let mut failures = Vec::new();
    let mut warnings = Vec::new();
    if targets.require_competitive_report_pass && competitive.status != "pass" {
        failures.push(format!(
            "competitive report status is `{}`; public proof requires `pass`",
            competitive.status
        ));
    }
    if targets.require_zero_retrieval_model_tokens && local_gate.retrieval_model_tokens != 0 {
        failures.push(format!(
            "retrieval_model_tokens {} exceeds required zero-token retrieval target",
            local_gate.retrieval_model_tokens
        ));
    }
    if targets.require_zero_per_query_retrieval_cost
        && local_gate.per_query_retrieval_cost_usd > f64::EPSILON
    {
        failures.push(format!(
            "per_query_retrieval_cost_usd {:.6} exceeds required zero-cost retrieval target",
            local_gate.per_query_retrieval_cost_usd
        ));
    }
    if targets.require_no_default_code_upload && local_gate.default_code_upload_required {
        failures.push("default_code_upload_required must be false".to_string());
    }
    if targets.require_broad_read_payload_reduction && broad_read_guardrail.status != "pass" {
        failures.push(format!(
            "broad-read guardrail status is `{}`; baseline payload {} tokens, CallSieve packet {} tokens, reduction {:.1}% below target {:.1}%",
            broad_read_guardrail.status,
            broad_read_guardrail.baseline_context_payload_tokens,
            broad_read_guardrail.callsieve_context_payload_tokens,
            broad_read_guardrail.context_payload_reduction_percent,
            broad_read_guardrail.minimum_context_payload_reduction_percent
        ));
    }
    if targets.require_context_packet_quality && context_packet_guardrail.status != "pass" {
        failures.push(format!(
            "context-packet guardrail status is `{}`; selected_files {}, selected_symbols {}, related_tests {}, blast_radius_hints {}, call_graph_hints {}, selection_reasons {}, selection_signals {}, focus_targets {}",
            context_packet_guardrail.status,
            context_packet_guardrail.selected_files,
            context_packet_guardrail.selected_symbols,
            context_packet_guardrail.related_tests,
            context_packet_guardrail.blast_radius_hints,
            context_packet_guardrail.call_graph_hints,
            context_packet_guardrail.selection_reasons,
            context_packet_guardrail.selection_signals,
            context_packet_guardrail.focus_targets
        ));
    }
    if agent_setup.status != "pass" {
        failures.push(format!(
            "agent setup coverage status is `{}`; public proof requires required setup clients to be covered",
            agent_setup.status
        ));
    }
    if let Some(max_packet_tokens) = targets.maximum_default_packet_tokens
        && local_gate.default_packet_tokens > max_packet_tokens
    {
        failures.push(format!(
            "default_packet_tokens {} exceeds public proof target {}",
            local_gate.default_packet_tokens, max_packet_tokens
        ));
    }

    let mut public_benchmarks = Vec::new();
    for input in &manifest.public_reports {
        let require_current_retrieval_contract = input
            .require_current_retrieval_contract
            .unwrap_or(targets.require_current_public_report_retrieval_contract);
        let evidence = public_benchmark_evidence(input, require_current_retrieval_contract)?;
        if evidence.retrieval_contract_status == "stale" {
            failures.push(format!(
                "public report `{}` retrieval_contract_fingerprint is stale",
                evidence.id
            ));
        } else if require_current_retrieval_contract
            && evidence.retrieval_contract_status != "current"
        {
            failures.push(format!(
                "public report `{}` retrieval_contract_fingerprint is `{}` but current is required",
                evidence.id, evidence.retrieval_contract_status
            ));
        }
        if let Some(minimum) = input.minimum_first_correct_file_rate_at_k
            && evidence.first_correct_file_rate_at_k < minimum
        {
            failures.push(format!(
                "public report `{}` {} first_correct_file_rate_at_k {:.3} is below target {:.3}",
                evidence.id, evidence.preferred_arm, evidence.first_correct_file_rate_at_k, minimum
            ));
        }
        if input.require_grep_baseline && evidence.grep_first_correct_file_rate_at_k.is_none() {
            failures.push(format!(
                "public report `{}` requires a grep baseline but none was recorded",
                evidence.id
            ));
        }
        if let Some(minimum) = input.minimum_minus_grep {
            match evidence.minus_grep {
                Some(actual) if actual >= minimum => {}
                Some(actual) => failures.push(format!(
                    "public report `{}` lift over grep {:.3} is below target {:.3}",
                    evidence.id, actual, minimum
                )),
                None => failures.push(format!(
                    "public report `{}` cannot prove lift over grep without a grep baseline",
                    evidence.id
                )),
            }
        }
        if let Some(minimum) = input.minimum_evaluated
            && evidence.evaluated < minimum
        {
            failures.push(format!(
                "public report `{}` evaluated {} issues, below target {}",
                evidence.id, evidence.evaluated, minimum
            ));
        }
        public_benchmarks.push(evidence);
    }

    let public_result_catalog = public_result_catalog(&manifest.public_result_catalog, &targets)?;
    for report in &public_result_catalog.reports {
        if report.retrieval_contract_status == "stale" {
            failures.push(format!(
                "public result catalog report `{}` retrieval_contract_fingerprint is stale",
                report.report
            ));
        }
    }
    let public_checkouts = public_checkout_evidence(
        &manifest.public_repos_dir,
        &public_benchmarks,
        &public_result_catalog,
    )?;
    if public_checkouts.status != "pass" {
        warnings.push(format!(
            "public benchmark checkouts are not rerun-ready: {} missing repo(s), {} missing manifest(s)",
            public_checkouts.missing_repos.len(),
            public_checkouts.missing_manifests.len()
        ));
    }
    if let Some(best) = best_measured_public_rate(
        &public_benchmarks,
        &public_result_catalog,
        "swe_bench_style",
    ) && best.0 < targets.target_swe_bench_style_first_correct_file_rate_at_k
    {
        warnings.push(format!(
            "SWE-bench-style public proof is {:.1}% at k={}, below the {:.1}% next target",
            best.0 * 100.0,
            best.1,
            targets.target_swe_bench_style_first_correct_file_rate_at_k * 100.0
        ));
    }
    if let Some(best) = best_measured_public_rate(
        &public_benchmarks,
        &public_result_catalog,
        "natural_language",
    ) && best.0 < targets.target_natural_language_first_correct_file_rate_at_k
    {
        warnings.push(format!(
            "natural-language public proof is {:.1}% at k={}, below the {:.1}% next target",
            best.0 * 100.0,
            best.1,
            targets.target_natural_language_first_correct_file_rate_at_k * 100.0
        ));
    }

    if public_benchmarks.len() < targets.minimum_public_reports {
        failures.push(format!(
            "public proof includes {} report(s), below target {}",
            public_benchmarks.len(),
            targets.minimum_public_reports
        ));
    }
    let total_public_evaluated: usize = public_benchmarks
        .iter()
        .map(|evidence| evidence.evaluated)
        .sum();
    if let Some(minimum) = targets.minimum_total_public_evaluated
        && total_public_evaluated < minimum
    {
        failures.push(format!(
            "public proof evaluated {} issues, below target {}",
            total_public_evaluated, minimum
        ));
    }

    let average_public_selected_files =
        weighted_average_selected_files(&public_benchmarks).unwrap_or(0.0);
    let repo_packer_baselines = public_repo_packer_baselines(
        &manifest.repo_packer_baselines,
        local_gate.default_packet_tokens,
    )?;
    for baseline in &repo_packer_baselines {
        if baseline.required && !baseline.exists {
            failures.push(format!(
                "required repo-packer baseline `{}` is missing at {}",
                baseline.id, baseline.path
            ));
        } else if baseline.required && !baseline.locally_measured {
            failures.push(format!(
                "required repo-packer baseline `{}` did not include a parseable token or byte count",
                baseline.id
            ));
        } else if baseline.required && baseline.status != "pass" {
            failures.push(format!(
                "required repo-packer baseline `{}` status is `{}`",
                baseline.id, baseline.status
            ));
        } else if !baseline.required && baseline.exists && baseline.status != "pass" {
            warnings.push(format!(
                "optional repo-packer baseline `{}` status is `{}`",
                baseline.id, baseline.status
            ));
        }
    }
    let packer_guardrail_pass = targets
        .maximum_default_packet_tokens
        .is_none_or(|target| local_gate.default_packet_tokens <= target)
        && average_public_selected_files > 0.0;
    let packer_baseline_pass = repo_packer_baselines
        .iter()
        .filter(|baseline| baseline.required)
        .all(|baseline| baseline.status == "pass");
    let measured_packer_baselines = repo_packer_baselines
        .iter()
        .filter(|baseline| baseline.locally_measured)
        .count();
    let repo_packer_guardrail = PublicRepoPackerGuardrail {
        status: if packer_guardrail_pass && packer_baseline_pass {
            "pass".to_string()
        } else {
            "needs_work".to_string()
        },
        claim_scope: if measured_packer_baselines > 0 {
            "Measured comparison: compact selected files and packet-token ceilings plus checked-in repo-packer or full-repo pack proxy output-size baselines.".to_string()
        } else {
            "Measured guardrail: compact selected files and packet-token ceilings. External repo-packer CLIs are optional baselines, not counted as locally measured unless their output is added to the manifest.".to_string()
        },
        default_packet_tokens: local_gate.default_packet_tokens,
        maximum_default_packet_tokens: targets.maximum_default_packet_tokens,
        average_public_selected_files,
        baselines: repo_packer_baselines.clone(),
        evidence: if measured_packer_baselines > 0 {
            "CallSieve reports top-k files from public tasks, keeps the local competitive packet under budget, and compares that packet against checked-in full-repo pack output metrics.".to_string()
        } else {
            "CallSieve reports top-k files from public tasks and keeps the local competitive packet under the manifest budget.".to_string()
        },
        next_baseline: if measured_packer_baselines > 0 {
            "Refresh checked-in pack baseline artifacts before publishing a release claim, and add external Repomix, Gitingest, or Code2Prompt artifacts when those tools are installed and approved.".to_string()
        } else {
            "Add optional local Repomix, Gitingest, or Code2Prompt output-size baselines when those tools are installed.".to_string()
        },
    };

    let agent_native_search_baselines =
        public_agent_native_search_baselines(&manifest.agent_native_search_baselines)?;
    for baseline in &agent_native_search_baselines {
        if baseline.required && !baseline.exists {
            failures.push(format!(
                "required agent-native search baseline `{}` is missing at {}",
                baseline.id, baseline.path
            ));
        } else if baseline.required && !baseline.locally_measured {
            failures.push(format!(
                "required agent-native search baseline `{}` was not marked locally measured",
                baseline.id
            ));
        } else if baseline.required && baseline.transcript_provenance_status != "pass" {
            failures.push(format!(
                "required agent-native search baseline `{}` lacks hashed transcript/export provenance",
                baseline.id
            ));
        } else if baseline.required && baseline.status != "pass" {
            failures.push(format!(
                "required agent-native search baseline `{}` status is `{}`",
                baseline.id, baseline.status
            ));
        } else if !baseline.required && baseline.exists && baseline.status != "pass" {
            warnings.push(format!(
                "optional agent-native search baseline `{}` status is `{}`",
                baseline.id, baseline.status
            ));
        }
    }
    let measured_agent_native_baselines = agent_native_search_baselines
        .iter()
        .filter(|baseline| baseline.locally_measured)
        .count();
    let agent_native_search_summary =
        public_agent_native_search_summary(&agent_native_search_baselines);
    let agent_native_target_pass =
        agent_native_search_summary_satisfies_targets(&agent_native_search_summary, &targets);
    if let Some(minimum) = targets.minimum_agent_native_measured_baselines
        && agent_native_search_summary.measured_baseline_count < minimum
    {
        failures.push(format!(
            "agent-native search baselines measured {} tools/runs, below minimum_agent_native_measured_baselines {}",
            agent_native_search_summary.measured_baseline_count, minimum
        ));
    }
    if let Some(minimum) = targets.minimum_agent_native_distinct_tools
        && agent_native_search_summary.distinct_agent_tool_count < minimum
    {
        failures.push(format!(
            "agent-native search baselines cover {} distinct agent tools, below minimum_agent_native_distinct_tools {}",
            agent_native_search_summary.distinct_agent_tool_count, minimum
        ));
    }
    if let Some(minimum) = targets.minimum_agent_native_repositories
        && agent_native_search_summary.distinct_repository_count < minimum
    {
        failures.push(format!(
            "agent-native search baselines cover {} repositories, below minimum_agent_native_repositories {}",
            agent_native_search_summary.distinct_repository_count, minimum
        ));
    }
    if let Some(minimum) = targets.minimum_agent_native_task_languages
        && agent_native_search_summary.task_language_count < minimum
    {
        failures.push(format!(
            "agent-native search baselines cover {} task languages, below minimum_agent_native_task_languages {}",
            agent_native_search_summary.task_language_count, minimum
        ));
    }
    if measured_agent_native_baselines > 0
        && agent_native_search_summary.distinct_agent_tool_count < 2
    {
        warnings.push(
            "agent-native proof is currently single-agent only; add another transcript-backed native-agent baseline before claiming multi-agent coverage"
                .to_string(),
        );
    }
    let agent_native_required_pass = agent_native_search_baselines
        .iter()
        .filter(|baseline| baseline.required)
        .all(|baseline| baseline.status == "pass");
    let agent_native_optional_pass = agent_native_search_baselines
        .iter()
        .filter(|baseline| !baseline.required && baseline.exists)
        .all(|baseline| baseline.status == "pass");
    let mut agent_native_search_guardrail = PublicAgentNativeSearchGuardrail {
        status: if agent_native_search_baselines.is_empty() {
            "not_measured".to_string()
        } else if agent_native_required_pass && agent_native_optional_pass && agent_native_target_pass
        {
            "pass".to_string()
        } else {
            "needs_work".to_string()
        },
        claim_scope: if measured_agent_native_baselines > 0 {
            "Measured comparison: CallSieve public proof includes checked agent-native search baseline artifacts with hashed transcript/export provenance, first-correct metrics, and context-token metrics.".to_string()
        } else {
            "Protocol-ready guardrail: no agent-native search baseline is checked in yet, so public proof does not claim a measured win over Cursor, Copilot, Claude Code, Devin, or other native codebase search.".to_string()
        },
        summary: agent_native_search_summary.clone(),
        baselines: agent_native_search_baselines.clone(),
        evidence: if measured_agent_native_baselines > 0 {
            "Agent-native baseline artifacts are parsed from local JSON and must include hashed task logs plus source transcripts or native-search exports, the checked agent-native measurement protocol, and a passing measured preflight artifact with matching source hashes before metric thresholds can pass.".to_string()
        } else {
            "Add measured artifacts from approved agent-native search runs before claiming head-to-head wins over agent-native codebase search.".to_string()
        },
        next_baseline: "Run `callsieve agent-native-protocol --out benchmarks/public/results/agent-native-protocol.json`, generate a template with `callsieve agent-native-template`, fill agent_native_files and transcript-backed agent_native_context_tokens from an approved native-search run, save the preflight with `callsieve agent-native-check --mode measured --out benchmarks/public/results/<tool>-check.json`, convert it with `callsieve agent-native-baseline`, then add the check under terminal_artifacts and the baseline under agent_native_search_baselines.".to_string(),
    };

    let mcp_surface = public_mcp_surface_evidence();
    if mcp_surface.status != "pass" {
        failures.push(format!(
            "MCP surface is missing required first-mile tools: {}",
            mcp_surface.missing_required_tools.join(", ")
        ));
    }
    let mcp_contract = public_mcp_contract_evidence();
    if mcp_contract.status != "pass" {
        failures.push(format!(
            "MCP context contract status is `{}`; public proof requires stable structuredContent fields",
            mcp_contract.status
        ));
    }

    let miss_triage = manifest
        .miss_triage
        .as_deref()
        .map(public_miss_triage)
        .transpose()?;
    let terminal_artifacts = public_artifact_evidence(&manifest.terminal_artifacts);
    for artifact in &terminal_artifacts {
        if artifact.required && !artifact.exists {
            failures.push(format!(
                "required public proof artifact `{}` is missing at {}",
                artifact.id, artifact.path
            ));
        } else if artifact.required && artifact.status != "pass" {
            failures.push(format!(
                "required public proof artifact `{}` status is `{}`: {}",
                artifact.id, artifact.status, artifact.validation
            ));
        } else if !artifact.required && artifact.exists && artifact.status != "pass" {
            warnings.push(format!(
                "optional public proof artifact `{}` status is `{}`: {}",
                artifact.id, artifact.status, artifact.validation
            ));
        }
    }
    let agent_native_protocol_artifact_pass = terminal_artifacts
        .iter()
        .any(|artifact| artifact.id == "agent-native-protocol" && artifact.status == "pass");
    let agent_native_check_artifact_pass = terminal_artifacts
        .iter()
        .any(|artifact| artifact.id == "agent-native-check" && artifact.status == "pass");
    let agent_native_check_source_artifact_hash_sets = terminal_artifacts
        .iter()
        .filter(|artifact| artifact.id == "agent-native-check" && artifact.status == "pass")
        .map(|artifact| {
            artifact
                .source_artifact_hashes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    if measured_agent_native_baselines > 0 && !agent_native_protocol_artifact_pass {
        failures.push(
            "measured agent-native search baselines require a passing `agent-native-protocol` public proof artifact".to_string(),
        );
        agent_native_search_guardrail.status = "needs_work".to_string();
    }
    if measured_agent_native_baselines > 0 && !agent_native_check_artifact_pass {
        failures.push(
            "measured agent-native search baselines require a passing `agent-native-check` public proof artifact".to_string(),
        );
        agent_native_search_guardrail.status = "needs_work".to_string();
    }
    if measured_agent_native_baselines > 0 && agent_native_check_artifact_pass {
        for baseline in agent_native_search_baselines
            .iter()
            .filter(|baseline| baseline.locally_measured)
        {
            let baseline_hashes = baseline
                .source_artifact_hashes
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            let linked_to_single_check = agent_native_check_source_artifact_hash_sets
                .iter()
                .any(|check_hashes| baseline_hashes.is_subset(check_hashes));
            if !linked_to_single_check {
                failures.push(format!(
                    "measured agent-native search baseline `{}` is not linked to a passing `agent-native-check` artifact containing all source artifact hashes: {}",
                    baseline.id,
                    baseline.source_artifact_hashes.join(", ")
                ));
                agent_native_search_guardrail.status = "needs_work".to_string();
            }
        }
    }
    let evidence_commands = if manifest.evidence_commands.is_empty() {
        default_public_proof_commands(manifest_path)
    } else {
        manifest.evidence_commands.clone()
    };
    let evidence_pack = public_evidence_pack(
        &local_gate,
        &targets,
        &public_benchmarks,
        &public_result_catalog,
        &evidence_commands,
        terminal_artifacts,
        &repo_packer_baselines,
        &agent_native_search_baselines,
        &agent_native_search_summary,
        &broad_read_guardrail,
        &context_packet_guardrail,
        &mcp_surface,
        &mcp_contract,
        &agent_setup,
        &public_checkouts,
    );

    let mut next_actions = Vec::new();
    if best_measured_public_rate(
        &public_benchmarks,
        &public_result_catalog,
        "swe_bench_style",
    )
    .is_none_or(|best| best.0 < targets.target_swe_bench_style_first_correct_file_rate_at_k)
    {
        next_actions.push(
            "raise SWE-bench-style public retrieval beyond the 60% first-correct-file@5 plateau"
                .to_string(),
        );
    }
    if best_measured_public_rate(
        &public_benchmarks,
        &public_result_catalog,
        "natural_language",
    )
    .is_none_or(|best| best.0 < targets.target_natural_language_first_correct_file_rate_at_k)
    {
        next_actions.push(
            "raise natural-language public retrieval toward the 45% first-correct-file@5 target"
                .to_string(),
        );
    }
    if !failures.is_empty() {
        next_actions.push(
            "fix failing public proof gates before using this artifact externally".to_string(),
        );
    }
    if public_checkouts.status != "pass" {
        next_actions.push(
            "populate public benchmark checkouts, then rerun bench-run from the reported commands"
                .to_string(),
        );
    }
    if broad_read_guardrail.status != "pass" {
        next_actions.push(
            "refresh the competitive fixture or context packet until public proof beats broad file reads"
                .to_string(),
        );
    }
    if context_packet_guardrail.status != "pass" {
        next_actions.push(
            "restore symbols, tests, blast-radius/call-graph hints, selection signals, and local expansion targets in the default context packet"
                .to_string(),
        );
    }
    next_actions.push(
        "rerun public benchmark reports from pinned local checkouts before making a release claim"
            .to_string(),
    );
    if measured_packer_baselines == 0 {
        next_actions.push(
            "add external Repomix, Gitingest, or Code2Prompt output-size baselines when those tools are installed and approved"
                .to_string(),
        );
    }
    if measured_agent_native_baselines == 0 {
        next_actions.push(
            "add measured agent-native search baselines from approved Cursor, Copilot, Claude Code, Devin, or similar runs"
                .to_string(),
        );
    } else if agent_native_search_summary.distinct_agent_tool_count < 2 {
        next_actions.push(
            "add a second transcript-backed native-agent baseline so public proof moves from Codex-only to real multi-agent coverage"
                .to_string(),
        );
    }

    Ok(PublicProofReportOutput {
        command: "public-proof-report",
        status: if failures.is_empty() {
            "pass".to_string()
        } else {
            "needs_work".to_string()
        },
        generated_at: now_unix_seconds(),
        goal: manifest.goal,
        local_gate,
        targets: PublicProofTargetsOutput {
            require_competitive_report_pass: targets.require_competitive_report_pass,
            require_zero_retrieval_model_tokens: targets.require_zero_retrieval_model_tokens,
            require_zero_per_query_retrieval_cost: targets.require_zero_per_query_retrieval_cost,
            require_no_default_code_upload: targets.require_no_default_code_upload,
            maximum_default_packet_tokens: targets.maximum_default_packet_tokens,
            require_broad_read_payload_reduction: targets.require_broad_read_payload_reduction,
            require_context_packet_quality: targets.require_context_packet_quality,
            require_current_public_report_retrieval_contract: targets
                .require_current_public_report_retrieval_contract,
            minimum_broad_read_context_payload_reduction_percent: targets
                .minimum_broad_read_context_payload_reduction_percent,
            minimum_public_reports: targets.minimum_public_reports,
            minimum_total_public_evaluated: targets.minimum_total_public_evaluated,
            minimum_agent_native_measured_baselines: targets
                .minimum_agent_native_measured_baselines,
            minimum_agent_native_distinct_tools: targets.minimum_agent_native_distinct_tools,
            minimum_agent_native_repositories: targets.minimum_agent_native_repositories,
            minimum_agent_native_task_languages: targets.minimum_agent_native_task_languages,
            target_swe_bench_style_first_correct_file_rate_at_k: targets
                .target_swe_bench_style_first_correct_file_rate_at_k,
            target_natural_language_first_correct_file_rate_at_k: targets
                .target_natural_language_first_correct_file_rate_at_k,
        },
        public_benchmarks,
        public_result_catalog,
        public_checkouts,
        broad_read_guardrail,
        context_packet_guardrail,
        evidence_pack,
        miss_triage,
        repo_packer_guardrail,
        agent_native_search_guardrail,
        mcp_surface,
        mcp_contract,
        agent_setup,
        evidence_commands,
        failures,
        warnings,
        next_actions,
    })
}

fn public_proof_local_gate(
    manifest_path: &Path,
    competitive: &CompetitiveReportOutput,
) -> PublicProofLocalGate {
    PublicProofLocalGate {
        manifest: manifest_path.display().to_string(),
        status: competitive.status.clone(),
        expected_file_recall: competitive.callsieve.expected_file_recall,
        first_correct_file_rate_at_k: competitive.callsieve.first_correct_file_rate_at_k,
        natural_language_first_correct_file_rate_at_k: competitive
            .natural_language
            .as_ref()
            .map(|evidence| evidence.first_correct_file_rate_at_k),
        retrieval_model_tokens: competitive.callsieve.retrieval_model_tokens,
        per_query_retrieval_cost_usd: competitive.callsieve.per_query_retrieval_cost_usd,
        default_code_upload_required: competitive.callsieve.default_code_upload_required,
        default_packet_tokens: competitive.callsieve.default_packet_tokens,
        first_query_p95_ms: competitive.callsieve.first_query_p95_ms,
        trace_or_receipt_proof_available: competitive.callsieve.trace_or_receipt_proof_available,
        agent_coverage_count: competitive.callsieve.agent_coverage_count,
    }
}

fn public_broad_read_guardrail(
    callsieve: &CompetitiveCallsieveEvidence,
    targets: &PublicProofTargets,
) -> PublicBroadReadGuardrail {
    let context_payload_tokens_saved = callsieve.baseline_context_payload_tokens as isize
        - callsieve.callsieve_context_payload_tokens as isize;
    let context_payload_reduction_percent =
        if callsieve.context_payload_reduction_percent.is_finite() {
            callsieve.context_payload_reduction_percent
        } else if callsieve.baseline_context_payload_tokens > 0 {
            (context_payload_tokens_saved as f64 / callsieve.baseline_context_payload_tokens as f64)
                * 100.0
        } else {
            0.0
        };
    let minimum_context_payload_reduction_percent =
        targets.minimum_broad_read_context_payload_reduction_percent;
    let pass = callsieve.baseline_context_payload_tokens > 0
        && callsieve.callsieve_context_payload_tokens > 0
        && context_payload_tokens_saved > 0
        && context_payload_reduction_percent >= minimum_context_payload_reduction_percent;

    PublicBroadReadGuardrail {
        status: metric_status(pass),
        claim_scope:
            "Measured comparison: naive broad file-read payload versus the CallSieve context packet from the local competitive benchmark."
                .to_string(),
        baseline_context_payload_tokens: callsieve.baseline_context_payload_tokens,
        callsieve_context_payload_tokens: callsieve.callsieve_context_payload_tokens,
        context_payload_tokens_saved,
        context_payload_reduction_percent,
        minimum_context_payload_reduction_percent,
        avoided_file_reads: callsieve.avoided_file_reads,
        avoided_grep_commands: callsieve.avoided_grep_commands,
        evidence:
            "Uses competitive-report context_payload_reduction estimates from the same task set as the local public-proof gate."
                .to_string(),
        next_baseline:
            "Keep broad-read payload comparisons in competitive-report, then raise the minimum reduction target as public task coverage grows."
                .to_string(),
    }
}

fn public_context_packet_guardrail(
    callsieve: &CompetitiveCallsieveEvidence,
) -> PublicContextPacketGuardrail {
    let quality = &callsieve.packet_quality;
    let has_selected_context = quality.tasks > 0 && quality.selected_files > 0;
    let has_symbol_context = quality.selected_symbols > 0 && quality.files_with_symbols > 0;
    let has_test_context = quality.related_tests > 0 && quality.files_with_related_tests > 0;
    let has_impact_context = quality.blast_radius_hints > 0 && quality.files_with_blast_radius > 0;
    let has_call_graph_context =
        quality.call_graph_hints > 0 && quality.files_with_call_graph_hints > 0;
    let has_risk_context = quality.files_with_non_unknown_risk > 0;
    let has_selection_evidence = quality.selection_reasons > 0
        && quality.files_with_selection_reasons > 0
        && quality.selection_signals > 0
        && quality.files_with_selection_confidence > 0;
    let has_snippets_or_local_expansion = quality.snippets > 0 || quality.focus_targets > 0;
    let has_followup_targets = quality.focus_targets > 0
        && quality.relationship_followup_targets > 0
        && quality.test_followup_targets > 0;
    let pass = has_selected_context
        && has_symbol_context
        && has_test_context
        && has_impact_context
        && has_call_graph_context
        && has_risk_context
        && has_selection_evidence
        && has_snippets_or_local_expansion
        && has_followup_targets;

    PublicContextPacketGuardrail {
        status: metric_status(pass),
        claim_scope:
            "Measured local packet-quality guardrail: the competitive benchmark context packets must contain selected files, symbols, related tests, blast-radius/risk hints, call-graph hints, ranking evidence, and local expansion targets before public proof can claim better-than-search context."
                .to_string(),
        tasks: quality.tasks,
        selected_files: quality.selected_files,
        files_with_symbols: quality.files_with_symbols,
        selected_symbols: quality.selected_symbols,
        files_with_snippets: quality.files_with_snippets,
        snippets: quality.snippets,
        files_with_related_tests: quality.files_with_related_tests,
        related_tests: quality.related_tests,
        files_with_blast_radius: quality.files_with_blast_radius,
        blast_radius_hints: quality.blast_radius_hints,
        files_with_call_graph_hints: quality.files_with_call_graph_hints,
        call_graph_hints: quality.call_graph_hints,
        files_with_non_unknown_risk: quality.files_with_non_unknown_risk,
        files_with_selection_reasons: quality.files_with_selection_reasons,
        selection_reasons: quality.selection_reasons,
        files_with_selection_confidence: quality.files_with_selection_confidence,
        selection_signals: quality.selection_signals,
        next_file_hints: quality.next_file_hints,
        focus_targets: quality.focus_targets,
        relationship_followup_targets: quality.relationship_followup_targets,
        test_followup_targets: quality.test_followup_targets,
        evidence:
            "Uses benchmark-report packet_quality counters from the same local competitive task set as the broad-read and zero-token gates."
                .to_string(),
        next_baseline:
            "Keep this guardrail passing as default packets get slimmer, and add transcript-backed agent-native comparisons before claiming native-search wins."
                .to_string(),
    }
}

fn public_benchmark_evidence(
    input: &PublicBenchmarkReportInput,
    require_current_retrieval_contract: bool,
) -> Result<PublicBenchmarkEvidence> {
    let report_json = fs::read_to_string(&input.path).with_context(|| {
        format!(
            "failed to read public benchmark report: {}",
            input.path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&report_json).with_context(|| {
        format!(
            "failed to parse public benchmark report: {}",
            input.path.display()
        )
    })?;
    let preferred_arm = input.preferred_arm.to_ascii_lowercase();
    if !matches!(preferred_arm.as_str(), "lexical" | "hybrid") {
        bail!(
            "public report `{}` has unsupported preferred_arm `{}` (expected lexical or hybrid)",
            input.id,
            input.preferred_arm
        );
    }
    let category = if input.category.trim().is_empty() {
        infer_public_report_category(&input.id, &value)
    } else {
        input.category.clone()
    };
    let (retrieval_contract_fingerprint, retrieval_contract_status) =
        public_report_retrieval_contract_status(&value);
    let first_correct_file_rate_at_k = public_report_arm_rate(&value, &preferred_arm);
    let grep_first_correct_file_rate_at_k =
        json_pointer_f64(&value, "/aggregate/grep_first_correct_file_rate_at_k");
    let minus_grep =
        grep_first_correct_file_rate_at_k.map(|grep| first_correct_file_rate_at_k - grep);
    let evaluated = json_pointer_usize(&value, "/aggregate/evaluated");
    let skipped = json_pointer_usize(&value, "/aggregate/skipped");
    let total = json_pointer_usize(&value, "/aggregate/total");
    let average_selected_files = average_selected_files_for_arm(&value, &preferred_arm);
    let average_grep_matched_files = if grep_first_correct_file_rate_at_k.is_some() {
        Some(average_selected_files_for_arm(&value, "grep"))
    } else {
        None
    };
    let misses = public_report_misses(&value, &preferred_arm, 8);
    let status = if input
        .minimum_first_correct_file_rate_at_k
        .is_none_or(|minimum| first_correct_file_rate_at_k >= minimum)
        && input
            .minimum_minus_grep
            .is_none_or(|minimum| minus_grep.is_some_and(|actual| actual >= minimum))
        && input
            .minimum_evaluated
            .is_none_or(|minimum| evaluated >= minimum)
        && (!input.require_grep_baseline || grep_first_correct_file_rate_at_k.is_some())
        && retrieval_contract_status != "stale"
        && (!require_current_retrieval_contract || retrieval_contract_status == "current")
    {
        "pass"
    } else {
        "needs_work"
    };

    Ok(PublicBenchmarkEvidence {
        id: input.id.clone(),
        category,
        report: input.path.display().to_string(),
        manifest: public_report_manifest_path(&value).unwrap_or_else(|| "unknown".into()),
        retrieval_contract_fingerprint,
        retrieval_contract_status,
        requires_current_retrieval_contract: require_current_retrieval_contract,
        k: json_pointer_usize(&value, "/k"),
        evaluated,
        skipped,
        total,
        preferred_arm,
        first_correct_file_rate_at_k,
        grep_first_correct_file_rate_at_k,
        minus_grep,
        average_selected_files,
        average_grep_matched_files,
        misses,
        status: status.to_string(),
    })
}

fn public_result_catalog(
    paths: &[PathBuf],
    targets: &PublicProofTargets,
) -> Result<PublicResultCatalog> {
    let mut reports = Vec::new();
    for path in paths {
        reports.push(public_result_catalog_entry(path)?);
    }
    let best_swe_bench_style = public_result_catalog_best(
        &reports,
        "swe_bench_style",
        targets.target_swe_bench_style_first_correct_file_rate_at_k,
    );
    let best_natural_language = public_result_catalog_best(
        &reports,
        "natural_language",
        targets.target_natural_language_first_correct_file_rate_at_k,
    );
    Ok(PublicResultCatalog {
        reports,
        best_swe_bench_style,
        best_natural_language,
    })
}

fn public_result_catalog_entry(path: &Path) -> Result<PublicResultCatalogEntry> {
    let report_json = fs::read_to_string(path).with_context(|| {
        format!(
            "failed to read public result catalog entry: {}",
            path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&report_json).with_context(|| {
        format!(
            "failed to parse public result catalog entry: {}",
            path.display()
        )
    })?;
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("public-result");
    let lexical_first_correct_file_rate_at_k = public_report_arm_rate(&value, "lexical");
    let hybrid_first_correct_file_rate_at_k = public_report_arm_rate(&value, "hybrid");
    let (best_arm, best_first_correct_file_rate_at_k) =
        if hybrid_first_correct_file_rate_at_k > lexical_first_correct_file_rate_at_k {
            ("hybrid".to_string(), hybrid_first_correct_file_rate_at_k)
        } else {
            ("lexical".to_string(), lexical_first_correct_file_rate_at_k)
        };
    let (retrieval_contract_fingerprint, retrieval_contract_status) =
        public_report_retrieval_contract_status(&value);
    Ok(PublicResultCatalogEntry {
        report: path.display().to_string(),
        manifest: public_report_manifest_path(&value).unwrap_or_else(|| "unknown".into()),
        category: infer_public_report_category(id, &value),
        retrieval_contract_fingerprint,
        retrieval_contract_status,
        k: json_pointer_usize(&value, "/k"),
        evaluated: json_pointer_usize(&value, "/aggregate/evaluated"),
        lexical_first_correct_file_rate_at_k,
        hybrid_first_correct_file_rate_at_k,
        grep_first_correct_file_rate_at_k: json_pointer_f64(
            &value,
            "/aggregate/grep_first_correct_file_rate_at_k",
        ),
        best_arm,
        best_first_correct_file_rate_at_k,
    })
}

fn public_result_catalog_best(
    reports: &[PublicResultCatalogEntry],
    category: &str,
    target: f64,
) -> Option<PublicResultCatalogBest> {
    reports
        .iter()
        .filter(|entry| entry.category == category)
        .max_by(|left, right| {
            left.best_first_correct_file_rate_at_k
                .partial_cmp(&right.best_first_correct_file_rate_at_k)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.evaluated.cmp(&right.evaluated))
        })
        .map(|entry| {
            let gap_to_target = (target - entry.best_first_correct_file_rate_at_k).max(0.0);
            PublicResultCatalogBest {
                report: entry.report.clone(),
                category: entry.category.clone(),
                best_arm: entry.best_arm.clone(),
                first_correct_file_rate_at_k: entry.best_first_correct_file_rate_at_k,
                target_first_correct_file_rate_at_k: target,
                gap_to_target,
                status: metric_status(gap_to_target <= f64::EPSILON),
            }
        })
}

fn public_checkout_evidence(
    repos_dir: &Path,
    public_benchmarks: &[PublicBenchmarkEvidence],
    public_result_catalog: &PublicResultCatalog,
) -> Result<PublicCheckoutEvidence> {
    let mut manifest_paths = BTreeSet::new();
    for evidence in public_benchmarks {
        manifest_paths.insert(evidence.manifest.clone());
    }
    for entry in &public_result_catalog.reports {
        manifest_paths.insert(entry.manifest.clone());
    }

    let mut repos = BTreeSet::new();
    let mut missing_manifests = Vec::new();
    for manifest_path in &manifest_paths {
        let path = Path::new(manifest_path);
        if !path.is_file() {
            missing_manifests.push(manifest_path.clone());
            continue;
        }
        let json = fs::read_to_string(path)
            .with_context(|| format!("failed to read public benchmark manifest {manifest_path}"))?;
        let manifest = bench_public::Manifest::from_str(&json).with_context(|| {
            format!("failed to parse public benchmark manifest {manifest_path}")
        })?;
        for issue in manifest.issues {
            repos.insert(issue.repo);
        }
    }
    missing_manifests.sort();

    let mut missing_repos = Vec::new();
    let mut present_repos = 0usize;
    for repo in &repos {
        if repos_dir.join(repo).is_dir() {
            present_repos += 1;
        } else {
            missing_repos.push(repo.clone());
        }
    }

    let status = if missing_repos.is_empty() && missing_manifests.is_empty() {
        "pass"
    } else {
        "needs_work"
    }
    .to_string();

    Ok(PublicCheckoutEvidence {
        status,
        repos_dir: repos_dir.display().to_string(),
        manifests: manifest_paths.into_iter().collect(),
        expected_repos: repos.len(),
        present_repos,
        missing_repos,
        missing_manifests,
        commands: public_checkout_commands(repos_dir),
    })
}

fn public_checkout_commands(repos_dir: &Path) -> Vec<String> {
    let repos_dir = repos_dir.display();
    vec![
        format!(
            "cargo run --features embed -- bench-run benchmarks/public/manifest-50.json --workdir {repos_dir} --compare --out benchmarks/public/results/compare-50-rerun.json --resume"
        ),
        format!(
            "cargo run --features embed -- bench-run benchmarks/public/manifest-50.json --workdir {repos_dir} --out benchmarks/public/results/mode-a-50-domain-rerun.json --resume"
        ),
        format!(
            "cargo run --features embed -- bench-run benchmarks/public/manifest.json --workdir {repos_dir} --out benchmarks/public/results/mode-a-requests-seed-rerun.json --resume"
        ),
        format!(
            "cargo run --features embed -- bench-run benchmarks/public/manifest-nl.json --workdir {repos_dir} --compare --out benchmarks/public/results/compare-nl-rerun.json --resume"
        ),
        format!(
            "cargo run --features embed -- bench-run benchmarks/public/manifest-nl.json --workdir {repos_dir} --out benchmarks/public/results/mode-a-nl-domain-rerun.json --resume"
        ),
        format!(
            "cargo run --features embed -- bench-run benchmarks/public/manifest-rust.json --workdir {repos_dir} --out benchmarks/public/results/mode-a-rust-callsieve-rerun.json --resume"
        ),
        format!(
            "cargo run --features embed -- bench-run benchmarks/public/manifest-typescript.json --workdir {repos_dir} --out benchmarks/public/results/mode-a-typescript-callsieve-rerun.json --resume"
        ),
    ]
}

fn best_measured_public_rate(
    public_benchmarks: &[PublicBenchmarkEvidence],
    public_result_catalog: &PublicResultCatalog,
    category: &str,
) -> Option<(f64, usize)> {
    let benchmark_best = public_benchmarks
        .iter()
        .filter(|evidence| evidence.category == category)
        .map(|evidence| (evidence.first_correct_file_rate_at_k, evidence.k))
        .max_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let catalog_best = public_result_catalog
        .reports
        .iter()
        .filter(|entry| entry.category == category)
        .map(|entry| (entry.best_first_correct_file_rate_at_k, entry.k))
        .max_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    match (benchmark_best, catalog_best) {
        (Some(left), Some(right)) => {
            if right.0 > left.0 {
                Some(right)
            } else {
                Some(left)
            }
        }
        (Some(best), None) | (None, Some(best)) => Some(best),
        (None, None) => None,
    }
}

fn public_repo_packer_baselines(
    inputs: &[PublicRepoPackerBaselineInput],
    callsieve_packet_tokens: usize,
) -> Result<Vec<PublicRepoPackerBaselineEvidence>> {
    inputs
        .iter()
        .map(|input| public_repo_packer_baseline(input, callsieve_packet_tokens))
        .collect()
}

fn repo_pack_baseline(path: &Path, id: String, tool: String) -> Result<RepoPackBaselineOutput> {
    let root = path
        .canonicalize()
        .with_context(|| format!("failed to resolve repo root {}", path.display()))?;
    let files = indexer::walker::source_files(&root)
        .with_context(|| format!("failed to walk repo {}", root.display()))?;
    let mut source_bytes = 0usize;
    let mut framing_bytes = 0usize;
    let mut largest_files = Vec::new();

    for relative in files {
        let absolute = root.join(&relative);
        let bytes = fs::read(&absolute)
            .with_context(|| format!("failed to read source file {}", absolute.display()))?;
        let relative_display = repo_relative_display(&root, &absolute);
        let header = format!("\n--- {relative_display} ---\n");
        source_bytes = source_bytes.saturating_add(bytes.len());
        framing_bytes = framing_bytes.saturating_add(header.len());
        largest_files.push(RepoPackBaselineFile {
            path: relative_display,
            bytes: bytes.len(),
            tokens: bytes.len().div_ceil(4),
        });
    }

    largest_files.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then(left.path.cmp(&right.path))
    });
    let file_count = largest_files.len();
    largest_files.truncate(12);
    let total_bytes = source_bytes.saturating_add(framing_bytes);

    Ok(RepoPackBaselineOutput {
        command: "repo-pack-baseline",
        schema_version: 1,
        id,
        tool,
        generated_at: now_unix_seconds(),
        root: root.display().to_string(),
        inclusion_scope: "gitignore-respecting source/config/docs files from the local checkout, measured as a full-repo prompt-pack proxy",
        token_estimate: "ceil(total_bytes / 4)",
        locally_measured: true,
        measurement_notes: vec![
            "No external repo-packer CLI was executed.",
            "This proxy measures the local prompt payload class that repo packers produce: file headers plus source/config/docs bytes.",
            "Use external Repomix, Gitingest, or Code2Prompt JSON artifacts when those tools are installed and approved.",
        ],
        metrics: RepoPackBaselineMetrics {
            file_count,
            source_bytes,
            framing_bytes,
            total_bytes,
            total_tokens: total_bytes.div_ceil(4),
        },
        largest_files,
    })
}

fn agent_native_baseline(
    tasks_path: &Path,
    id: String,
    tool: String,
    k: usize,
    measurement_command: String,
    source_artifacts: Vec<PathBuf>,
    measurement_notes: Vec<String>,
) -> Result<AgentNativeBaselineOutput> {
    if k == 0 {
        bail!("agent-native baseline requires --k to be greater than zero");
    }
    if source_artifacts.is_empty() {
        bail!(
            "agent-native baseline requires at least one --source-artifact transcript or native-search export"
        );
    }

    let tasks_json = fs::read_to_string(tasks_path).with_context(|| {
        format!(
            "failed to read agent-native baseline tasks: {}",
            tasks_path.display()
        )
    })?;
    let input: AgentNativeBaselineInput = serde_json::from_str(&tasks_json).with_context(|| {
        format!(
            "failed to parse agent-native baseline tasks: {}",
            tasks_path.display()
        )
    })?;
    let tasks = input.into_tasks();
    if tasks.is_empty() {
        bail!("agent-native baseline requires at least one task record");
    }

    let mut task_evidence = Vec::new();
    let mut agent_native_hits = 0usize;
    let mut callsieve_hits = 0usize;
    let mut agent_native_total_context_tokens = 0usize;
    let mut callsieve_total_packet_tokens = 0usize;

    for task in tasks {
        if task.expected_files.is_empty() {
            bail!(
                "agent-native baseline task `{}` must include at least one expected file",
                task.id
            );
        }
        if task.agent_native_context_tokens == 0 {
            bail!(
                "agent-native baseline task `{}` must include agent_native_context_tokens > 0",
                task.id
            );
        }
        if task.callsieve_packet_tokens == 0 {
            bail!(
                "agent-native baseline task `{}` must include callsieve_packet_tokens > 0",
                task.id
            );
        }
        if task.recording_status.as_deref() != Some("measured") {
            bail!(
                "agent-native baseline task `{}` must set recording_status to measured",
                task.id
            );
        }

        let agent_native_hit = bench_public::first_correct_file_rate_at_k(
            &task.agent_native_files,
            &task.expected_files,
            k,
        ) > 0.0;
        let callsieve_hit = bench_public::first_correct_file_rate_at_k(
            &task.callsieve_files,
            &task.expected_files,
            k,
        ) > 0.0;
        agent_native_hits += usize::from(agent_native_hit);
        callsieve_hits += usize::from(callsieve_hit);
        agent_native_total_context_tokens =
            agent_native_total_context_tokens.saturating_add(task.agent_native_context_tokens);
        callsieve_total_packet_tokens =
            callsieve_total_packet_tokens.saturating_add(task.callsieve_packet_tokens);
        task_evidence.push(AgentNativeBaselineTaskEvidence {
            id: task.id,
            repo: task.repo,
            base_commit: task.base_commit,
            task: task.task,
            expected_files: task.expected_files,
            agent_native_files: task.agent_native_files,
            callsieve_files: task.callsieve_files,
            callsieve_index_fingerprint: task.callsieve_index_fingerprint,
            agent_native_first_correct_file_at_k: agent_native_hit,
            callsieve_first_correct_file_at_k: callsieve_hit,
            agent_native_context_tokens: task.agent_native_context_tokens,
            callsieve_packet_tokens: task.callsieve_packet_tokens,
        });
    }

    let task_count = task_evidence.len();
    let agent_native_first_correct_file_rate_at_k = agent_native_hits as f64 / task_count as f64;
    let callsieve_first_correct_file_rate_at_k = callsieve_hits as f64 / task_count as f64;
    let agent_native_average_context_tokens =
        agent_native_total_context_tokens.div_ceil(task_count);
    let callsieve_average_packet_tokens = callsieve_total_packet_tokens.div_ceil(task_count);

    let mut source_artifact_evidence = Vec::with_capacity(source_artifacts.len() + 1);
    source_artifact_evidence.push(AgentNativeSourceArtifactEvidence {
        path: tasks_path.display().to_string(),
        role: "task_log",
        bytes: tasks_json.len(),
        hash: indexer::stable_content_hash(tasks_json.as_bytes()),
    });
    for source_artifact in &source_artifacts {
        source_artifact_evidence.push(agent_native_source_artifact_evidence(
            source_artifact,
            "agent_native_transcript_or_export",
        )?);
    }
    let source_artifact_paths = source_artifact_evidence
        .iter()
        .map(|artifact| artifact.path.clone())
        .collect();

    let mut notes = vec![
        "Generated from local task-level records of approved agent-native search runs.".to_string(),
        "CallSieve did not execute or simulate the external agent; it only computed metrics from recorded files and token counts.".to_string(),
        "Transcript provenance is backed by hashed local source artifacts.".to_string(),
    ];
    notes.extend(measurement_notes);

    Ok(AgentNativeBaselineOutput {
        command: "agent-native-baseline",
        schema_version: 1,
        id,
        tool,
        generated_at: now_unix_seconds(),
        locally_measured: true,
        measurement_scope: "operator-recorded agent-native codebase-search files and token counts compared with CallSieve read-first packets",
        measurement_command,
        source_artifacts: source_artifact_paths,
        transcript_provenance_status: "pass",
        source_artifact_evidence,
        measurement_notes: notes,
        k,
        metrics: AgentNativeBaselineMetrics {
            task_count,
            agent_native_first_correct_file_hits: agent_native_hits,
            callsieve_first_correct_file_hits: callsieve_hits,
            agent_native_first_correct_file_rate_at_k,
            callsieve_first_correct_file_rate_at_k,
            callsieve_minus_agent_native_first_correct_file_rate_at_k:
                callsieve_first_correct_file_rate_at_k - agent_native_first_correct_file_rate_at_k,
            agent_native_total_context_tokens,
            callsieve_total_packet_tokens,
            agent_native_average_context_tokens,
            callsieve_average_packet_tokens,
            agent_native_context_token_ratio_vs_callsieve: agent_native_average_context_tokens
                as f64
                / callsieve_average_packet_tokens as f64,
        },
        tasks: task_evidence,
    })
}

fn agent_native_check(
    tasks_path: &Path,
    mode: AgentNativeCheckMode,
    source_artifacts: Vec<PathBuf>,
) -> Result<AgentNativeCheckOutput> {
    let tasks_json = fs::read_to_string(tasks_path).with_context(|| {
        format!(
            "failed to read agent-native check tasks: {}",
            tasks_path.display()
        )
    })?;
    let input: AgentNativeBaselineInput = serde_json::from_str(&tasks_json).with_context(|| {
        format!(
            "failed to parse agent-native check tasks: {}",
            tasks_path.display()
        )
    })?;
    let tasks = input.into_tasks();
    let mut issues = Vec::new();
    if tasks.is_empty() {
        issues.push(AgentNativeCheckIssue {
            task: None,
            field: "tasks",
            message: "task log must contain at least one task".to_string(),
        });
    }

    let mut tasks_with_expected_files = 0usize;
    let mut tasks_with_callsieve_files = 0usize;
    let mut tasks_with_callsieve_packet_tokens = 0usize;
    let mut tasks_with_agent_native_files = 0usize;
    let mut tasks_with_agent_native_context_tokens = 0usize;
    let mut tasks_ready_for_mode = 0usize;

    for task in &tasks {
        let task_id = Some(task.id.clone());
        let has_expected_files = !task.expected_files.is_empty();
        let has_callsieve_files = !task.callsieve_files.is_empty();
        let has_callsieve_packet_tokens = task.callsieve_packet_tokens > 0;
        let has_agent_native_files = !task.agent_native_files.is_empty();
        let has_agent_native_context_tokens = task.agent_native_context_tokens > 0;

        tasks_with_expected_files += usize::from(has_expected_files);
        tasks_with_callsieve_files += usize::from(has_callsieve_files);
        tasks_with_callsieve_packet_tokens += usize::from(has_callsieve_packet_tokens);
        tasks_with_agent_native_files += usize::from(has_agent_native_files);
        tasks_with_agent_native_context_tokens += usize::from(has_agent_native_context_tokens);

        if !has_expected_files {
            issues.push(AgentNativeCheckIssue {
                task: task_id.clone(),
                field: "expected_files",
                message: "task must include at least one expected file".to_string(),
            });
        }
        if !has_callsieve_files {
            issues.push(AgentNativeCheckIssue {
                task: task_id.clone(),
                field: "callsieve_files",
                message:
                    "task must include the CallSieve read-first files from agent-native-template"
                        .to_string(),
            });
        }
        if !has_callsieve_packet_tokens {
            issues.push(AgentNativeCheckIssue {
                task: task_id.clone(),
                field: "callsieve_packet_tokens",
                message: "task must include CallSieve packet tokens from agent-native-template"
                    .to_string(),
            });
        }
        if let Some(base_commit) = &task.base_commit
            && base_commit.trim().is_empty()
        {
            issues.push(AgentNativeCheckIssue {
                task: task_id.clone(),
                field: "base_commit",
                message: "base_commit, when present, must not be empty".to_string(),
            });
        }
        if let Some(fingerprint) = &task.callsieve_index_fingerprint
            && !fingerprint.starts_with("fnv1a64:")
        {
            issues.push(AgentNativeCheckIssue {
                task: task_id.clone(),
                field: "callsieve_index_fingerprint",
                message: "callsieve_index_fingerprint must use fnv1a64 when present".to_string(),
            });
        }

        let mode_ready = match mode {
            AgentNativeCheckMode::Template => {
                if has_agent_native_files {
                    issues.push(AgentNativeCheckIssue {
                        task: task_id.clone(),
                        field: "agent_native_files",
                        message: "template mode must leave agent_native_files empty".to_string(),
                    });
                }
                if has_agent_native_context_tokens {
                    issues.push(AgentNativeCheckIssue {
                        task: task_id.clone(),
                        field: "agent_native_context_tokens",
                        message: "template mode must leave agent_native_context_tokens at 0"
                            .to_string(),
                    });
                }
                if task.recording_status.as_deref() != Some("needs_agent_native_measurement") {
                    issues.push(AgentNativeCheckIssue {
                        task: task_id.clone(),
                        field: "recording_status",
                        message: "template mode requires recording_status to be needs_agent_native_measurement".to_string(),
                    });
                }
                !has_agent_native_files
                    && !has_agent_native_context_tokens
                    && task.recording_status.as_deref() == Some("needs_agent_native_measurement")
            }
            AgentNativeCheckMode::Measured => {
                let measured_status = task.recording_status.as_deref() == Some("measured");
                if !has_agent_native_files {
                    issues.push(AgentNativeCheckIssue {
                        task: task_id.clone(),
                        field: "agent_native_files",
                        message:
                            "measured mode must include the files the native agent selected or read"
                                .to_string(),
                    });
                }
                if !has_agent_native_context_tokens {
                    issues.push(AgentNativeCheckIssue {
                        task: task_id.clone(),
                        field: "agent_native_context_tokens",
                        message: "measured mode must include transcript-backed native-agent context tokens".to_string(),
                    });
                }
                if !measured_status {
                    issues.push(AgentNativeCheckIssue {
                        task: task_id.clone(),
                        field: "recording_status",
                        message: "measured mode requires recording_status to be measured"
                            .to_string(),
                    });
                }
                has_agent_native_files && has_agent_native_context_tokens && measured_status
            }
        };
        if has_expected_files && has_callsieve_files && has_callsieve_packet_tokens && mode_ready {
            tasks_ready_for_mode += 1;
        }
    }

    let mut source_artifact_evidence = vec![AgentNativeSourceArtifactEvidence {
        path: tasks_path.display().to_string(),
        role: "task_log",
        bytes: tasks_json.len(),
        hash: indexer::stable_content_hash(tasks_json.as_bytes()),
    }];
    if mode == AgentNativeCheckMode::Measured && source_artifacts.is_empty() {
        issues.push(AgentNativeCheckIssue {
            task: None,
            field: "source_artifact",
            message: "measured mode requires at least one transcript or native-search export source artifact".to_string(),
        });
    }
    for source_artifact in source_artifacts {
        match agent_native_source_artifact_evidence(
            &source_artifact,
            "agent_native_transcript_or_export",
        ) {
            Ok(evidence) => source_artifact_evidence.push(evidence),
            Err(err) => issues.push(AgentNativeCheckIssue {
                task: None,
                field: "source_artifact",
                message: err.to_string(),
            }),
        }
    }

    let status = metric_status(issues.is_empty());
    let next_step = if status == "pass" {
        match mode {
            AgentNativeCheckMode::Template => "Run the approved native-search measurement, fill agent_native_files and transcript-backed agent_native_context_tokens, set recording_status to measured, then run agent-native-check --mode measured.".to_string(),
            AgentNativeCheckMode::Measured => "Run agent-native-baseline with the same task log and source artifacts, then add the generated artifact to agent_native_search_baselines.".to_string(),
        }
    } else {
        "Fix every issue before using this task log for a public agent-native search claim."
            .to_string()
    };

    Ok(AgentNativeCheckOutput {
        command: "agent-native-check",
        schema_version: 1,
        status,
        mode,
        tasks_path: tasks_path.display().to_string(),
        task_count: tasks.len(),
        tasks_with_expected_files,
        tasks_with_callsieve_files,
        tasks_with_callsieve_packet_tokens,
        tasks_with_agent_native_files,
        tasks_with_agent_native_context_tokens,
        tasks_ready_for_mode,
        source_artifact_evidence,
        issues,
        next_step,
    })
}

fn agent_native_source_artifact_evidence(
    source_artifact: &Path,
    role: &'static str,
) -> Result<AgentNativeSourceArtifactEvidence> {
    if !source_artifact.is_file() {
        bail!(
            "agent-native source artifact is missing or not a file: {}",
            source_artifact.display()
        );
    }
    let bytes = fs::read(source_artifact).with_context(|| {
        format!(
            "failed to read agent-native source artifact: {}",
            source_artifact.display()
        )
    })?;
    Ok(AgentNativeSourceArtifactEvidence {
        path: source_artifact.display().to_string(),
        role,
        bytes: bytes.len(),
        hash: indexer::stable_content_hash(&bytes),
    })
}

fn ensure_agent_native_template_checkout_is_clean(path: &Path) -> Result<()> {
    let output = ProcessCommand::new("git")
        .arg("-C")
        .arg(path)
        .arg("status")
        .arg("--porcelain")
        .output()
        .with_context(|| {
            format!(
                "failed to inspect git status before agent-native template checkout: {}",
                path.display()
            )
        })?;
    if !output.status.success() {
        bail!(
            "git status failed before agent-native template checkout in {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let file = line.get(3..).unwrap_or(line).trim();
        if file == ".callsieve" || file == ".callsieve/" || file.starts_with(".callsieve/") {
            continue;
        }
        bail!(
            "agent-native template pinned checkout requires a clean worktree; dirty path `{file}` in {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_agent_native_template_repo_matches_path(path: &Path, repo: &str) -> Result<()> {
    let (owner, name) = repo.split_once('/').ok_or_else(|| {
        anyhow::anyhow!("invalid repo slug `{repo}` in agent-native template task")
    })?;
    let mut components = path
        .components()
        .rev()
        .filter_map(|component| component.as_os_str().to_str());
    let path_name = components.next().unwrap_or_default();
    let path_owner = components.next().unwrap_or_default();
    if path_owner != owner || path_name != name {
        bail!(
            "agent-native template task repo `{repo}` does not match template root {}",
            path.display()
        );
    }
    Ok(())
}

fn checkout_agent_native_template_commit(
    path: &Path,
    task_id: &str,
    base_commit: &str,
) -> Result<()> {
    let output = ProcessCommand::new("git")
        .arg("-C")
        .arg(path)
        .arg("checkout")
        .arg("-f")
        .arg(base_commit)
        .output()
        .with_context(|| {
            format!(
                "failed to check out base_commit {base_commit} for agent-native template task {task_id}"
            )
        })?;
    if !output.status.success() {
        bail!(
            "git checkout -f {base_commit} failed for agent-native template task {task_id} in {}\nstdout:\n{}\nstderr:\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn agent_native_template(
    path: &Path,
    tasks_path: &Path,
    options: AgentNativeTemplateOptions,
) -> Result<AgentNativeTemplateOutput> {
    if options.k == 0 {
        bail!("agent-native template requires --k to be greater than zero");
    }

    let tasks_json = fs::read_to_string(tasks_path).with_context(|| {
        format!(
            "failed to read agent-native template tasks: {}",
            tasks_path.display()
        )
    })?;
    let input: AgentNativeTemplateInput = serde_json::from_str(&tasks_json).with_context(|| {
        format!(
            "failed to parse agent-native template tasks: {}",
            tasks_path.display()
        )
    })?;
    let tasks = input.into_tasks();
    if tasks.is_empty() {
        bail!("agent-native template requires at least one task record");
    }

    let pinned_base_commits = tasks.iter().any(|task| task.base_commit.is_some());
    if pinned_base_commits {
        ensure_agent_native_template_checkout_is_clean(path)?;
    }
    let mut shared_index = None;
    let mut shared_refreshed = false;
    let mut shared_index_fingerprint = None;
    if !pinned_base_commits {
        let (index, _index_load_ms, refreshed) = load_fresh_index_timed(path)?;
        shared_refreshed = refreshed;
        shared_index_fingerprint = Some(indexer::index_fingerprint(&index));
        shared_index = Some(index);
    }
    let retrieval_contract_fingerprint = query::retrieval_contract_fingerprint();
    let source_tasks_hash = indexer::stable_content_hash(tasks_json.as_bytes());
    let view_options = query::ContextViewOptions {
        profile: options.profile.into(),
        token_budget: Some(options.token_budget),
        include_git: false,
        include_call_paths: false,
    };
    let mut output_tasks = Vec::with_capacity(tasks.len());
    let mut task_index_fingerprint_material = Vec::new();
    let mut rebuilt_any_index = false;

    for (index_in_suite, task) in tasks.into_iter().enumerate() {
        let id = task
            .id
            .unwrap_or_else(|| format!("task-{}", index_in_suite + 1));
        if let Some(repo) = &task.repo {
            ensure_agent_native_template_repo_matches_path(path, repo)?;
        }
        if let Some(base_commit) = &task.base_commit {
            checkout_agent_native_template_commit(path, &id, base_commit)?;
        }
        let expected_files = if task.expected_files.is_empty() {
            task.critical_files
        } else {
            task.expected_files
        };
        if expected_files.is_empty() {
            bail!("agent-native template task `{id}` must include expected_files");
        }

        let (index, task_index_fingerprint) = if pinned_base_commits {
            let (index, _index_load_ms, refreshed) = load_fresh_index_timed(path)?;
            rebuilt_any_index |= refreshed;
            let fingerprint = indexer::index_fingerprint(&index);
            (index, fingerprint)
        } else {
            (
                shared_index
                    .as_ref()
                    .expect("shared index exists when not pinned")
                    .clone(),
                shared_index_fingerprint
                    .as_ref()
                    .expect("shared index fingerprint exists when not pinned")
                    .clone(),
            )
        };
        let context = query::build_context_with_options(
            path,
            &index,
            &task.task,
            options.limit,
            options.snippets_per_file,
            options.include_snippets,
            false,
        )?;
        let context_value = query::context_value(&context, view_options)?;
        let callsieve_files = context_read_first_files(&context_value);
        let callsieve_packet_tokens = query::value_estimated_tokens(&context_value)?;
        let callsieve_first_correct_file_at_k = bench_public::first_correct_file_rate_at_k(
            &callsieve_files,
            &expected_files,
            options.k,
        ) > 0.0;
        task_index_fingerprint_material.push(format!(
            "{id}\0{}\0{task_index_fingerprint}",
            task.base_commit.as_deref().unwrap_or("")
        ));

        output_tasks.push(AgentNativeTemplateTaskOutput {
            id,
            repo: task.repo,
            base_commit: task.base_commit,
            task: task.task,
            expected_files,
            agent_native_files: Vec::new(),
            callsieve_files,
            agent_native_context_tokens: 0,
            callsieve_packet_tokens,
            callsieve_index_fingerprint: task_index_fingerprint,
            callsieve_first_correct_file_at_k,
            recording_status: "needs_agent_native_measurement",
        });
    }
    let index_fingerprint = if pinned_base_commits {
        indexer::stable_content_hash(task_index_fingerprint_material.join("\n").as_bytes())
    } else {
        shared_index_fingerprint.expect("shared index fingerprint exists when not pinned")
    };

    let mut warnings = Vec::new();
    if shared_refreshed || rebuilt_any_index {
        warnings.push("rebuilt missing or stale CallSieve index before templating".to_string());
    }
    if pinned_base_commits {
        warnings.push(
            "checked out pinned base_commit values while generating per-task CallSieve template"
                .to_string(),
        );
    }

    Ok(AgentNativeTemplateOutput {
        command: "agent-native-template",
        schema_version: 1,
        root: root_label(path),
        pinned_base_commits,
        index_fingerprint,
        retrieval_contract_fingerprint,
        source_tasks: tasks_path.display().to_string(),
        source_tasks_hash,
        locally_measured: false,
        status: "needs_agent_native_measurement",
        k: options.k,
        limit: options.limit,
        snippets_per_file: if options.include_snippets {
            options.snippets_per_file
        } else {
            0
        },
        profile: profile_arg_name(options.profile),
        token_budget: options.token_budget,
        instructions: vec![
            "Run the same tasks in the external agent using its native codebase search.",
            "Record the actual first files or search results the agent used in agent_native_files.",
            "Record transcript-backed context tokens in agent_native_context_tokens; do not estimate.",
            "Then pass this filled file to callsieve agent-native-baseline.",
        ],
        warnings,
        tasks: output_tasks,
    })
}

fn public_repo_packer_baseline(
    input: &PublicRepoPackerBaselineInput,
    callsieve_packet_tokens: usize,
) -> Result<PublicRepoPackerBaselineEvidence> {
    let path = input.path.display().to_string();
    let tool = if input.tool.trim().is_empty() {
        input.id.clone()
    } else {
        input.tool.clone()
    };
    if !input.path.is_file() {
        return Ok(PublicRepoPackerBaselineEvidence {
            id: input.id.clone(),
            tool,
            path,
            command: input.command.clone(),
            required: input.required,
            exists: false,
            locally_measured: false,
            tokens: None,
            tokens_estimated_from_bytes: false,
            bytes: None,
            files: None,
            token_ratio_vs_callsieve_packet: None,
            minimum_token_ratio_vs_callsieve_packet: input.minimum_token_ratio_vs_callsieve_packet,
            status: "needs_work".to_string(),
        });
    }

    let artifact_json = fs::read_to_string(&input.path).with_context(|| {
        format!(
            "failed to read repo-packer baseline artifact: {}",
            input.path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&artifact_json).with_context(|| {
        format!(
            "failed to parse repo-packer baseline artifact: {}",
            input.path.display()
        )
    })?;
    let bytes = json_metric_usize(
        &value,
        input.byte_count_pointer.as_deref(),
        &[
            "/bytes",
            "/byte_count",
            "/total_bytes",
            "/metrics/bytes",
            "/metrics/byte_count",
            "/metrics/total_bytes",
            "/summary/bytes",
            "/summary/byte_count",
            "/summary/total_bytes",
            "/stats/bytes",
            "/stats/byte_count",
            "/stats/total_bytes",
        ],
    );
    let direct_tokens = json_metric_usize(
        &value,
        input.token_count_pointer.as_deref(),
        &[
            "/tokens",
            "/token_count",
            "/total_tokens",
            "/metrics/tokens",
            "/metrics/token_count",
            "/metrics/total_tokens",
            "/summary/tokens",
            "/summary/token_count",
            "/summary/total_tokens",
            "/stats/tokens",
            "/stats/token_count",
            "/stats/total_tokens",
        ],
    );
    let tokens_estimated_from_bytes = direct_tokens.is_none() && bytes.is_some();
    let tokens = direct_tokens.or_else(|| bytes.map(|byte_count| byte_count.div_ceil(4)));
    let files = json_metric_usize(
        &value,
        input.file_count_pointer.as_deref(),
        &[
            "/files",
            "/file_count",
            "/total_files",
            "/metrics/files",
            "/metrics/file_count",
            "/metrics/total_files",
            "/summary/files",
            "/summary/file_count",
            "/summary/total_files",
            "/stats/files",
            "/stats/file_count",
            "/stats/total_files",
        ],
    );
    let token_ratio_vs_callsieve_packet = tokens.and_then(|tokens| {
        if callsieve_packet_tokens == 0 {
            None
        } else {
            Some(tokens as f64 / callsieve_packet_tokens as f64)
        }
    });
    let locally_measured = tokens.is_some() || bytes.is_some();
    let status = metric_status(
        locally_measured
            && input
                .minimum_token_ratio_vs_callsieve_packet
                .is_none_or(|minimum| {
                    token_ratio_vs_callsieve_packet.is_some_and(|ratio| ratio >= minimum)
                }),
    );

    Ok(PublicRepoPackerBaselineEvidence {
        id: input.id.clone(),
        tool,
        path,
        command: input.command.clone(),
        required: input.required,
        exists: true,
        locally_measured,
        tokens,
        tokens_estimated_from_bytes,
        bytes,
        files,
        token_ratio_vs_callsieve_packet,
        minimum_token_ratio_vs_callsieve_packet: input.minimum_token_ratio_vs_callsieve_packet,
        status,
    })
}

fn public_agent_native_search_baselines(
    inputs: &[PublicAgentNativeSearchBaselineInput],
) -> Result<Vec<PublicAgentNativeSearchBaselineEvidence>> {
    inputs
        .iter()
        .map(public_agent_native_search_baseline)
        .collect()
}

fn public_agent_native_search_baseline(
    input: &PublicAgentNativeSearchBaselineInput,
) -> Result<PublicAgentNativeSearchBaselineEvidence> {
    let path = input.path.display().to_string();
    let tool = if input.tool.trim().is_empty() {
        input.id.clone()
    } else {
        input.tool.clone()
    };
    if !input.path.is_file() {
        return Ok(PublicAgentNativeSearchBaselineEvidence {
            id: input.id.clone(),
            tool,
            path,
            command: input.command.clone(),
            required: input.required,
            exists: false,
            locally_measured: false,
            tasks: None,
            repositories: Vec::new(),
            base_commits: Vec::new(),
            task_languages: Vec::new(),
            agent_native_first_correct_file_rate_at_k: None,
            callsieve_first_correct_file_rate_at_k: None,
            callsieve_minus_agent_native_first_correct_file_rate_at_k: None,
            agent_native_average_context_tokens: None,
            callsieve_average_packet_tokens: None,
            agent_native_context_token_ratio_vs_callsieve: None,
            source_artifacts: None,
            source_artifact_hashes: Vec::new(),
            transcript_source_artifact_hashes: Vec::new(),
            transcript_provenance_status: "missing".to_string(),
            minimum_tasks: input.minimum_tasks,
            minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k: input
                .minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k,
            minimum_agent_native_context_token_ratio_vs_callsieve: input
                .minimum_agent_native_context_token_ratio_vs_callsieve,
            status: "needs_work".to_string(),
        });
    }

    let artifact_json = fs::read_to_string(&input.path).with_context(|| {
        format!(
            "failed to read agent-native search baseline artifact: {}",
            input.path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&artifact_json).with_context(|| {
        format!(
            "failed to parse agent-native search baseline artifact: {}",
            input.path.display()
        )
    })?;
    let locally_measured = value
        .pointer(&input.locally_measured_pointer)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let tasks = value
        .pointer(&input.task_count_pointer)
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let agent_native_first_correct_file_rate_at_k = json_pointer_f64(
        &value,
        &input.agent_native_first_correct_file_rate_at_k_pointer,
    );
    let callsieve_first_correct_file_rate_at_k = json_pointer_f64(
        &value,
        &input.callsieve_first_correct_file_rate_at_k_pointer,
    );
    let callsieve_minus_agent_native_first_correct_file_rate_at_k = match (
        callsieve_first_correct_file_rate_at_k,
        agent_native_first_correct_file_rate_at_k,
    ) {
        (Some(callsieve), Some(agent_native)) => Some(callsieve - agent_native),
        _ => None,
    };
    let agent_native_average_context_tokens = value
        .pointer(&input.agent_native_average_context_tokens_pointer)
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let callsieve_average_packet_tokens = value
        .pointer(&input.callsieve_average_packet_tokens_pointer)
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize);
    let agent_native_context_token_ratio_vs_callsieve = match (
        agent_native_average_context_tokens,
        callsieve_average_packet_tokens,
    ) {
        (Some(agent_native), Some(callsieve)) if callsieve > 0 => {
            Some(agent_native as f64 / callsieve as f64)
        }
        _ => None,
    };
    let (
        source_artifacts,
        source_artifact_hashes,
        transcript_source_artifact_hashes,
        source_artifact_provenance_pass,
    ) = agent_native_source_artifact_provenance(&value, &input.path);
    let (repositories, base_commits, task_languages) = agent_native_task_coverage(&value);
    let status = metric_status(
        locally_measured
            && source_artifact_provenance_pass
            && input
                .minimum_tasks
                .is_none_or(|minimum| tasks.is_some_and(|tasks| tasks >= minimum))
            && input
                .minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k
                .is_none_or(|minimum| {
                    callsieve_minus_agent_native_first_correct_file_rate_at_k
                        .is_some_and(|delta| delta >= minimum)
                })
            && input
                .minimum_agent_native_context_token_ratio_vs_callsieve
                .is_none_or(|minimum| {
                    agent_native_context_token_ratio_vs_callsieve
                        .is_some_and(|ratio| ratio >= minimum)
                }),
    );

    Ok(PublicAgentNativeSearchBaselineEvidence {
        id: input.id.clone(),
        tool,
        path,
        command: input.command.clone(),
        required: input.required,
        exists: true,
        locally_measured,
        tasks,
        repositories,
        base_commits,
        task_languages,
        agent_native_first_correct_file_rate_at_k,
        callsieve_first_correct_file_rate_at_k,
        callsieve_minus_agent_native_first_correct_file_rate_at_k,
        agent_native_average_context_tokens,
        callsieve_average_packet_tokens,
        agent_native_context_token_ratio_vs_callsieve,
        source_artifacts,
        source_artifact_hashes,
        transcript_source_artifact_hashes,
        transcript_provenance_status: metric_status(source_artifact_provenance_pass),
        minimum_tasks: input.minimum_tasks,
        minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k: input
            .minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k,
        minimum_agent_native_context_token_ratio_vs_callsieve: input
            .minimum_agent_native_context_token_ratio_vs_callsieve,
        status,
    })
}

fn public_agent_native_search_summary(
    baselines: &[PublicAgentNativeSearchBaselineEvidence],
) -> PublicAgentNativeSearchSummary {
    let measured_baselines = baselines
        .iter()
        .filter(|baseline| baseline.locally_measured)
        .collect::<Vec<_>>();
    let agent_tools = sorted_unique(
        measured_baselines
            .iter()
            .map(|baseline| baseline.tool.trim())
            .filter(|tool| !tool.is_empty()),
    );
    let repositories = sorted_unique(
        measured_baselines
            .iter()
            .flat_map(|baseline| baseline.repositories.iter().map(String::as_str)),
    );
    let base_commits = sorted_unique(
        measured_baselines
            .iter()
            .flat_map(|baseline| baseline.base_commits.iter().map(String::as_str)),
    );
    let task_languages = sorted_unique(
        measured_baselines
            .iter()
            .flat_map(|baseline| baseline.task_languages.iter().map(String::as_str)),
    );
    let measured_baseline_count = measured_baselines.len();
    let distinct_agent_tool_count = agent_tools.len();
    let single_agent_only = measured_baseline_count > 0 && distinct_agent_tool_count == 1;
    let multi_agent_status = if measured_baseline_count == 0 {
        "not_measured"
    } else if distinct_agent_tool_count > 1 {
        "pass"
    } else {
        "single_agent_only"
    }
    .to_string();

    PublicAgentNativeSearchSummary {
        baseline_count: baselines.len(),
        required_baseline_count: baselines
            .iter()
            .filter(|baseline| baseline.required)
            .count(),
        measured_baseline_count,
        transcript_backed_baseline_count: baselines
            .iter()
            .filter(|baseline| {
                baseline.locally_measured && baseline.transcript_provenance_status == "pass"
            })
            .count(),
        total_measured_tasks: measured_baselines
            .iter()
            .filter_map(|baseline| baseline.tasks)
            .sum(),
        distinct_agent_tool_count,
        agent_tools,
        distinct_repository_count: repositories.len(),
        repositories,
        distinct_base_commit_count: base_commits.len(),
        base_commits,
        task_language_count: task_languages.len(),
        task_languages,
        single_agent_only,
        multi_agent_status,
        evidence: if measured_baseline_count == 0 {
            "No measured agent-native baseline artifacts are checked in yet.".to_string()
        } else if single_agent_only {
            "Measured agent-native artifacts are transcript-backed, but they currently cover only one native agent tool.".to_string()
        } else {
            "Measured agent-native artifacts cover multiple native agent tools with transcript-backed source provenance.".to_string()
        },
        next_baseline: if distinct_agent_tool_count < 2 {
            "Add a second real native-agent baseline with hashed transcript/export provenance."
                .to_string()
        } else {
            "Keep adding public repos, languages, and native agents while preserving hashed transcript/export provenance.".to_string()
        },
    }
}

fn agent_native_search_summary_satisfies_targets(
    summary: &PublicAgentNativeSearchSummary,
    targets: &PublicProofTargets,
) -> bool {
    targets
        .minimum_agent_native_measured_baselines
        .is_none_or(|minimum| summary.measured_baseline_count >= minimum)
        && targets
            .minimum_agent_native_distinct_tools
            .is_none_or(|minimum| summary.distinct_agent_tool_count >= minimum)
        && targets
            .minimum_agent_native_repositories
            .is_none_or(|minimum| summary.distinct_repository_count >= minimum)
        && targets
            .minimum_agent_native_task_languages
            .is_none_or(|minimum| summary.task_language_count >= minimum)
}

fn sorted_unique<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn agent_native_task_coverage(
    value: &serde_json::Value,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) else {
        return (Vec::new(), Vec::new(), Vec::new());
    };

    let mut repositories = BTreeSet::new();
    let mut base_commits = BTreeSet::new();
    let mut task_languages = BTreeSet::new();
    for task in tasks {
        if let Some(repo) = task.get("repo").and_then(serde_json::Value::as_str)
            && !repo.trim().is_empty()
        {
            repositories.insert(repo.trim().to_string());
        }
        if let Some(base_commit) = task.get("base_commit").and_then(serde_json::Value::as_str)
            && !base_commit.trim().is_empty()
        {
            base_commits.insert(base_commit.trim().to_string());
        }

        let mut task_paths = json_string_array_key(task, "expected_files");
        if task_paths.is_empty() {
            task_paths.extend(json_string_array_key(task, "agent_native_files"));
            task_paths.extend(json_string_array_key(task, "callsieve_files"));
        }
        for path in task_paths {
            if let Some(language) = public_task_language_from_path(&path) {
                task_languages.insert(language.to_string());
            }
        }
    }

    (
        repositories.into_iter().collect(),
        base_commits.into_iter().collect(),
        task_languages.into_iter().collect(),
    )
}

fn json_string_array_key(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn public_task_language_from_path(path: &str) -> Option<&'static str> {
    match Path::new(path).extension().and_then(OsStr::to_str) {
        Some("py" | "pyi") => Some("Python"),
        Some("rs") => Some("Rust"),
        Some("ts" | "tsx") => Some("TypeScript"),
        Some("js" | "jsx" | "mjs" | "cjs") => Some("JavaScript"),
        _ => None,
    }
}

fn agent_native_source_artifact_provenance(
    value: &serde_json::Value,
    baseline_artifact_path: &Path,
) -> (Option<usize>, Vec<String>, Vec<String>, bool) {
    let Some(artifacts) = value
        .get("source_artifact_evidence")
        .and_then(serde_json::Value::as_array)
    else {
        return (None, Vec::new(), Vec::new(), false);
    };

    let mut source_artifact_hashes = Vec::new();
    let mut transcript_source_artifact_hashes = Vec::new();
    let mut has_transcript_or_export = false;
    let mut all_artifacts_valid = !artifacts.is_empty();
    for artifact in artifacts {
        let source_path = artifact
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let path_valid = !source_path.trim().is_empty();
        let role = artifact
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let declared_bytes = artifact
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let declared_bytes_valid = declared_bytes > 0;
        let hash = artifact
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let hash_valid = hash.starts_with("fnv1a64:") && hash.len() > "fnv1a64:".len();
        let source_artifact_path =
            resolve_agent_native_source_artifact_path(baseline_artifact_path, source_path);
        let source_artifact_bytes = fs::read(&source_artifact_path).ok();
        let bytes_match = source_artifact_bytes
            .as_ref()
            .is_some_and(|bytes| declared_bytes == bytes.len() as u64);
        let hash_match = source_artifact_bytes
            .as_ref()
            .is_some_and(|bytes| indexer::stable_content_hash(bytes) == hash);
        if hash_valid && hash_match {
            source_artifact_hashes.push(hash.to_string());
        }
        if role == "agent_native_transcript_or_export" {
            has_transcript_or_export = true;
            if hash_valid && hash_match {
                transcript_source_artifact_hashes.push(hash.to_string());
            }
        }
        all_artifacts_valid &= path_valid
            && !role.trim().is_empty()
            && declared_bytes_valid
            && hash_valid
            && bytes_match
            && hash_match;
    }

    let artifact_status_pass = value
        .get("transcript_provenance_status")
        .and_then(serde_json::Value::as_str)
        == Some("pass");
    (
        Some(artifacts.len()),
        source_artifact_hashes,
        transcript_source_artifact_hashes,
        all_artifacts_valid && has_transcript_or_export && artifact_status_pass,
    )
}

fn resolve_agent_native_source_artifact_path(
    baseline_artifact_path: &Path,
    source_path: &str,
) -> PathBuf {
    let path = Path::new(source_path);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if let Some(parent) = baseline_artifact_path.parent() {
        let candidate = parent.join(path);
        if candidate.is_file() {
            return candidate;
        }
    }
    path.to_path_buf()
}

fn public_mcp_surface_evidence() -> PublicMcpSurfaceEvidence {
    let exposed_tools = mcp::listed_tool_names();
    let exposed: BTreeSet<&str> = exposed_tools.iter().map(String::as_str).collect();
    let required_first_mile_tools: Vec<String> = mcp::FIRST_MILE_MCP_TOOLS
        .iter()
        .map(|tool| (*tool).to_string())
        .collect();
    let missing_required_tools: Vec<String> = mcp::FIRST_MILE_MCP_TOOLS
        .iter()
        .filter(|tool| !exposed.contains(**tool))
        .map(|tool| (*tool).to_string())
        .collect();

    PublicMcpSurfaceEvidence {
        status: if missing_required_tools.is_empty() {
            "pass".to_string()
        } else {
            "needs_work".to_string()
        },
        protocol: "2025-06-18",
        required_first_mile_tools,
        exposed_tools,
        missing_required_tools,
        context_first_tool: "callsieve_context",
        local_config_commands: vec![
            "cargo run -- mcp-config . --format json",
            "cargo run -- mcp-config . --format toml",
        ],
        registry_manifest_command: "cargo run -- mcp-registry-manifest --out server.json",
        evidence: "The public proof checks the actual MCP tools/list surface for context-first retrieval, targeted focus, related-file expansion, related tests, and freshness status before claiming MCP readiness.".to_string(),
    }
}

fn public_mcp_contract_evidence() -> PublicMcpContractEvidence {
    let exposed_tools = mcp::listed_tool_names();
    let context_tool_listed = exposed_tools.iter().any(|tool| tool == "callsieve_context");
    let required_structured_content_fields = mcp::CONTEXT_STRUCTURED_CONTENT_FIELDS.to_vec();
    let supported_instruction_expansion_keys = mcp::CONTEXT_INSTRUCTION_EXPANSION_KEYS.to_vec();
    let required_freshness_fields = mcp::CONTEXT_FRESHNESS_FIELDS.to_vec();
    let status = context_tool_listed
        && !required_structured_content_fields.is_empty()
        && !supported_instruction_expansion_keys.is_empty()
        && !required_freshness_fields.is_empty()
        && query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET > 0
        && mcp::CONTEXT_DEFAULT_PROFILE == "skim";

    PublicMcpContractEvidence {
        status: metric_status(status),
        contract_version: mcp::CONTEXT_STRUCTURED_CONTENT_CONTRACT_VERSION,
        context_tool: "callsieve_context",
        structured_content_required: true,
        default_profile: mcp::CONTEXT_DEFAULT_PROFILE,
        default_token_budget: query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET,
        required_structured_content_fields,
        supported_instruction_expansion_keys,
        required_freshness_fields,
        evidence: "MCP context proof exposes the stable structuredContent contract agents consume before broad grep or repeated file reads.",
    }
}

fn public_agent_setup_evidence(competitive: &CompetitiveReportOutput) -> PublicAgentSetupEvidence {
    let covered_agents: Vec<String> = competitive
        .callsieve
        .agent_coverage
        .iter()
        .map(|agent| (*agent).to_string())
        .collect();
    let covered: BTreeSet<String> = competitive
        .callsieve
        .agent_coverage
        .iter()
        .map(|agent| normalized_agent_name(agent))
        .collect();
    let priority_clients = public_proof_priority_setup_clients();
    let priority_clients_covered: Vec<&'static str> = priority_clients
        .iter()
        .copied()
        .filter(|agent| covered.contains(&normalized_agent_name(agent)))
        .collect();
    let default_layer_clients = competitive.targets.required_agents.clone();
    let default_layer_clients_covered: Vec<String> = default_layer_clients
        .iter()
        .filter(|agent| covered.contains(&normalized_agent_name(agent)))
        .cloned()
        .collect();
    let missing_default_layer_clients: Vec<String> = default_layer_clients
        .iter()
        .filter(|agent| !covered.contains(&normalized_agent_name(agent)))
        .cloned()
        .collect();
    let status = metric_status(
        competitive.callsieve.agent_coverage_count >= competitive.targets.minimum_agent_coverage
            && competitive.callsieve.missing_required_agents.is_empty()
            && missing_default_layer_clients.is_empty()
            && default_layer_clients_covered.len() == default_layer_clients.len()
            && priority_clients_covered.len() == priority_clients.len(),
    );

    PublicAgentSetupEvidence {
        status,
        claim_scope: "Agent-agnostic local setup evidence: setup-auto detection, project-local MCP config, registry descriptor, lifecycle hooks where available, and templates where hooks are not available.",
        required_agents: competitive.targets.required_agents.clone(),
        covered_agents,
        missing_required_agents: competitive.callsieve.missing_required_agents.clone(),
        agent_coverage_count: competitive.callsieve.agent_coverage_count,
        minimum_agent_coverage: competitive.targets.minimum_agent_coverage,
        default_layer_clients,
        default_layer_clients_covered,
        missing_default_layer_clients,
        priority_clients,
        priority_clients_covered,
        hook_capable_clients: vec![
            "Codex",
            "Claude Code",
            "GitHub Copilot",
            "OpenCode",
            "Antigravity",
            "Cline",
        ],
        mcp_template_clients: vec![
            "Cursor",
            "VS Code",
            "Windsurf",
            "Continue",
            "Zed",
            "Junie",
            "JetBrains",
            "Amp",
            "Goose",
            "Warp",
            "Zoo",
            "Roo",
            "generic MCP",
        ],
        commands: public_agent_setup_commands(),
        evidence: "The competitive gate requires agent coverage, and public proof exposes the exact setup and MCP commands needed to reproduce the local integration surface.",
        next_step: "Run setup-auto --dry-run first, review detected clients, then run setup-auto or per-client hook/MCP setup in the target repository.",
    }
}

fn public_proof_priority_setup_clients() -> Vec<&'static str> {
    vec![
        "Codex",
        "Claude Code",
        "GitHub Copilot",
        "Cursor",
        "OpenCode",
        "Zed",
        "generic MCP",
    ]
}

fn public_agent_setup_commands() -> Vec<String> {
    vec![
        "cargo run -- setup-auto . --dry-run".to_string(),
        "cargo run -- setup-auto .".to_string(),
        "cargo run -- mcp-config . --format json".to_string(),
        "cargo run -- mcp-config . --format toml".to_string(),
        "cargo run -- mcp-registry-manifest --out server.json".to_string(),
        "cargo run -- mcp-contract --out benchmarks/public/results/mcp-contract.json".to_string(),
        "cargo run -- agent-native-protocol --out benchmarks/public/results/agent-native-protocol.json".to_string(),
        "cargo run -- agent-native-template benchmarks/public/repos/psf/requests benchmarks/public/manifest.json --k 5 --out benchmarks/public/results/agent-native-requests-template.json".to_string(),
        "cargo run -- codex-hooks doctor . --strict --smoke".to_string(),
        "cargo run -- claude-hooks doctor . --strict".to_string(),
        "cargo run -- opencode-hooks doctor . --strict".to_string(),
    ]
}

#[allow(clippy::too_many_arguments)]
fn public_evidence_pack(
    local_gate: &PublicProofLocalGate,
    targets: &PublicProofTargets,
    public_benchmarks: &[PublicBenchmarkEvidence],
    public_result_catalog: &PublicResultCatalog,
    commands: &[String],
    terminal_artifacts: Vec<PublicProofArtifactEvidence>,
    repo_packer_baselines: &[PublicRepoPackerBaselineEvidence],
    agent_native_search_baselines: &[PublicAgentNativeSearchBaselineEvidence],
    agent_native_search_summary: &PublicAgentNativeSearchSummary,
    broad_read_guardrail: &PublicBroadReadGuardrail,
    context_packet_guardrail: &PublicContextPacketGuardrail,
    mcp_surface: &PublicMcpSurfaceEvidence,
    mcp_contract: &PublicMcpContractEvidence,
    agent_setup: &PublicAgentSetupEvidence,
    public_checkouts: &PublicCheckoutEvidence,
) -> PublicEvidencePack {
    let mut fixture_manifests: Vec<String> = public_benchmarks
        .iter()
        .map(|evidence| evidence.manifest.clone())
        .collect();
    fixture_manifests.push(local_gate.manifest.clone());
    fixture_manifests.sort();
    fixture_manifests.dedup();

    let mut result_reports: Vec<String> = public_benchmarks
        .iter()
        .map(|evidence| evidence.report.clone())
        .collect();
    result_reports.extend(
        public_result_catalog
            .reports
            .iter()
            .map(|entry| entry.report.clone()),
    );
    result_reports.extend(
        repo_packer_baselines
            .iter()
            .map(|baseline| baseline.path.clone()),
    );
    result_reports.extend(
        agent_native_search_baselines
            .iter()
            .map(|baseline| baseline.path.clone()),
    );
    result_reports.sort();
    result_reports.dedup();

    let total_public_evaluated: usize = public_benchmarks
        .iter()
        .map(|evidence| evidence.evaluated)
        .sum();
    let mut metrics = vec![
        PublicProofMetric {
            name: "local.expected_file_recall".to_string(),
            value: serde_json::json!(local_gate.expected_file_recall),
            target: Some(serde_json::json!(1.0)),
            status: metric_status(local_gate.expected_file_recall >= 1.0),
        },
        PublicProofMetric {
            name: "local.retrieval_model_tokens".to_string(),
            value: serde_json::json!(local_gate.retrieval_model_tokens),
            target: Some(serde_json::json!(0)),
            status: metric_status(local_gate.retrieval_model_tokens == 0),
        },
        PublicProofMetric {
            name: "local.per_query_retrieval_cost_usd".to_string(),
            value: serde_json::json!(local_gate.per_query_retrieval_cost_usd),
            target: Some(serde_json::json!(0.0)),
            status: metric_status(local_gate.per_query_retrieval_cost_usd <= f64::EPSILON),
        },
        PublicProofMetric {
            name: "local.default_code_upload_required".to_string(),
            value: serde_json::json!(local_gate.default_code_upload_required),
            target: Some(serde_json::json!(false)),
            status: metric_status(!local_gate.default_code_upload_required),
        },
        PublicProofMetric {
            name: "public.total_evaluated".to_string(),
            value: serde_json::json!(total_public_evaluated),
            target: targets
                .minimum_total_public_evaluated
                .map(|target| serde_json::json!(target)),
            status: metric_status(
                targets
                    .minimum_total_public_evaluated
                    .is_none_or(|target| total_public_evaluated >= target),
            ),
        },
        PublicProofMetric {
            name: "mcp_surface.exposed_tools".to_string(),
            value: serde_json::json!(mcp_surface.exposed_tools.len()),
            target: Some(serde_json::json!(
                mcp_surface.required_first_mile_tools.len()
            )),
            status: metric_status(
                mcp_surface.exposed_tools.len() >= mcp_surface.required_first_mile_tools.len(),
            ),
        },
        PublicProofMetric {
            name: "mcp_surface.required_first_mile_tools_covered".to_string(),
            value: serde_json::json!(
                mcp_surface.required_first_mile_tools.len()
                    - mcp_surface.missing_required_tools.len()
            ),
            target: Some(serde_json::json!(
                mcp_surface.required_first_mile_tools.len()
            )),
            status: metric_status(mcp_surface.missing_required_tools.is_empty()),
        },
        PublicProofMetric {
            name: "mcp_contract.required_structured_content_fields".to_string(),
            value: serde_json::json!(mcp_contract.required_structured_content_fields.len()),
            target: Some(serde_json::json!(
                mcp::CONTEXT_STRUCTURED_CONTENT_FIELDS.len()
            )),
            status: mcp_contract.status.clone(),
        },
        PublicProofMetric {
            name: "mcp_contract.supported_instruction_expansion_keys".to_string(),
            value: serde_json::json!(mcp_contract.supported_instruction_expansion_keys.len()),
            target: Some(serde_json::json!(
                mcp::CONTEXT_INSTRUCTION_EXPANSION_KEYS.len()
            )),
            status: mcp_contract.status.clone(),
        },
        PublicProofMetric {
            name: "mcp_contract.required_freshness_fields".to_string(),
            value: serde_json::json!(mcp_contract.required_freshness_fields.len()),
            target: Some(serde_json::json!(mcp::CONTEXT_FRESHNESS_FIELDS.len())),
            status: mcp_contract.status.clone(),
        },
        PublicProofMetric {
            name: "mcp_contract.default_token_budget".to_string(),
            value: serde_json::json!(mcp_contract.default_token_budget),
            target: Some(serde_json::json!(query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET)),
            status: mcp_contract.status.clone(),
        },
        PublicProofMetric {
            name: "agent_setup.agent_coverage_count".to_string(),
            value: serde_json::json!(agent_setup.agent_coverage_count),
            target: Some(serde_json::json!(agent_setup.minimum_agent_coverage)),
            status: metric_status(
                agent_setup.agent_coverage_count >= agent_setup.minimum_agent_coverage,
            ),
        },
        PublicProofMetric {
            name: "agent_setup.priority_clients_covered".to_string(),
            value: serde_json::json!(agent_setup.priority_clients_covered.len()),
            target: Some(serde_json::json!(agent_setup.priority_clients.len())),
            status: metric_status(
                agent_setup.priority_clients_covered.len() == agent_setup.priority_clients.len(),
            ),
        },
        PublicProofMetric {
            name: "agent_setup.default_layer_clients_covered".to_string(),
            value: serde_json::json!(agent_setup.default_layer_clients_covered.len()),
            target: Some(serde_json::json!(agent_setup.default_layer_clients.len())),
            status: metric_status(
                agent_setup.default_layer_clients_covered.len()
                    == agent_setup.default_layer_clients.len()
                    && agent_setup.missing_default_layer_clients.is_empty(),
            ),
        },
        PublicProofMetric {
            name: "agent_setup.missing_required_agents".to_string(),
            value: serde_json::json!(agent_setup.missing_required_agents.len()),
            target: Some(serde_json::json!(0)),
            status: metric_status(agent_setup.missing_required_agents.is_empty()),
        },
        PublicProofMetric {
            name: "public_checkouts.missing_repos".to_string(),
            value: serde_json::json!(public_checkouts.missing_repos.len()),
            target: Some(serde_json::json!(0)),
            status: metric_status(public_checkouts.missing_repos.is_empty()),
        },
        PublicProofMetric {
            name: "public_checkouts.missing_manifests".to_string(),
            value: serde_json::json!(public_checkouts.missing_manifests.len()),
            target: Some(serde_json::json!(0)),
            status: metric_status(public_checkouts.missing_manifests.is_empty()),
        },
        PublicProofMetric {
            name: "broad_read.baseline_context_payload_tokens".to_string(),
            value: serde_json::json!(broad_read_guardrail.baseline_context_payload_tokens),
            target: Some(serde_json::json!(1)),
            status: metric_status(broad_read_guardrail.baseline_context_payload_tokens > 0),
        },
        PublicProofMetric {
            name: "broad_read.callsieve_context_payload_tokens".to_string(),
            value: serde_json::json!(broad_read_guardrail.callsieve_context_payload_tokens),
            target: Some(serde_json::json!(1)),
            status: metric_status(broad_read_guardrail.callsieve_context_payload_tokens > 0),
        },
        PublicProofMetric {
            name: "broad_read.context_payload_tokens_saved".to_string(),
            value: serde_json::json!(broad_read_guardrail.context_payload_tokens_saved),
            target: Some(serde_json::json!(1)),
            status: metric_status(broad_read_guardrail.context_payload_tokens_saved > 0),
        },
        PublicProofMetric {
            name: "broad_read.context_payload_reduction_percent".to_string(),
            value: serde_json::json!(broad_read_guardrail.context_payload_reduction_percent),
            target: Some(serde_json::json!(
                broad_read_guardrail.minimum_context_payload_reduction_percent
            )),
            status: broad_read_guardrail.status.clone(),
        },
        PublicProofMetric {
            name: "broad_read.avoided_file_reads".to_string(),
            value: serde_json::json!(broad_read_guardrail.avoided_file_reads),
            target: None,
            status: broad_read_guardrail.status.clone(),
        },
        PublicProofMetric {
            name: "broad_read.avoided_grep_commands".to_string(),
            value: serde_json::json!(broad_read_guardrail.avoided_grep_commands),
            target: None,
            status: broad_read_guardrail.status.clone(),
        },
        PublicProofMetric {
            name: "context_packet.selected_files".to_string(),
            value: serde_json::json!(context_packet_guardrail.selected_files),
            target: Some(serde_json::json!(1)),
            status: metric_status(context_packet_guardrail.selected_files > 0),
        },
        PublicProofMetric {
            name: "context_packet.selected_symbols".to_string(),
            value: serde_json::json!(context_packet_guardrail.selected_symbols),
            target: Some(serde_json::json!(1)),
            status: metric_status(context_packet_guardrail.selected_symbols > 0),
        },
        PublicProofMetric {
            name: "context_packet.related_tests".to_string(),
            value: serde_json::json!(context_packet_guardrail.related_tests),
            target: Some(serde_json::json!(1)),
            status: metric_status(context_packet_guardrail.related_tests > 0),
        },
        PublicProofMetric {
            name: "context_packet.blast_radius_hints".to_string(),
            value: serde_json::json!(context_packet_guardrail.blast_radius_hints),
            target: Some(serde_json::json!(1)),
            status: metric_status(context_packet_guardrail.blast_radius_hints > 0),
        },
        PublicProofMetric {
            name: "context_packet.call_graph_hints".to_string(),
            value: serde_json::json!(context_packet_guardrail.call_graph_hints),
            target: Some(serde_json::json!(1)),
            status: metric_status(context_packet_guardrail.call_graph_hints > 0),
        },
        PublicProofMetric {
            name: "context_packet.selection_evidence".to_string(),
            value: serde_json::json!({
                "selection_reasons": context_packet_guardrail.selection_reasons,
                "selection_signals": context_packet_guardrail.selection_signals,
                "files_with_selection_confidence": context_packet_guardrail.files_with_selection_confidence
            }),
            target: Some(serde_json::json!({
                "selection_reasons": 1,
                "selection_signals": 1,
                "files_with_selection_confidence": 1
            })),
            status: metric_status(
                context_packet_guardrail.selection_reasons > 0
                    && context_packet_guardrail.selection_signals > 0
                    && context_packet_guardrail.files_with_selection_confidence > 0,
            ),
        },
        PublicProofMetric {
            name: "context_packet.local_expansion_targets".to_string(),
            value: serde_json::json!({
                "focus_targets": context_packet_guardrail.focus_targets,
                "relationship_followup_targets": context_packet_guardrail.relationship_followup_targets,
                "test_followup_targets": context_packet_guardrail.test_followup_targets
            }),
            target: Some(serde_json::json!({
                "focus_targets": 1,
                "relationship_followup_targets": 1,
                "test_followup_targets": 1
            })),
            status: metric_status(
                context_packet_guardrail.focus_targets > 0
                    && context_packet_guardrail.relationship_followup_targets > 0
                    && context_packet_guardrail.test_followup_targets > 0,
            ),
        },
    ];
    if let Some(artifact) = terminal_artifacts
        .iter()
        .find(|artifact| artifact.id == "mcp-contract")
    {
        metrics.push(PublicProofMetric {
            name: "mcp_contract.artifact_matches_live_contract".to_string(),
            value: serde_json::json!(artifact.status == "pass"),
            target: Some(serde_json::json!(true)),
            status: artifact.status.clone(),
        });
    }
    if let Some(artifact) = terminal_artifacts
        .iter()
        .find(|artifact| artifact.id == "agent-native-template")
    {
        metrics.push(PublicProofMetric {
            name: "agent_native_template.ready_for_measurement".to_string(),
            value: serde_json::json!(artifact.status == "pass"),
            target: Some(serde_json::json!(true)),
            status: artifact.status.clone(),
        });
    }
    if let Some(artifact) = terminal_artifacts
        .iter()
        .find(|artifact| artifact.id == "agent-native-protocol")
    {
        metrics.push(PublicProofMetric {
            name: "agent_native_protocol.artifact_matches_live_protocol".to_string(),
            value: serde_json::json!(artifact.status == "pass"),
            target: Some(serde_json::json!(true)),
            status: artifact.status.clone(),
        });
    }
    if let Some(artifact) = terminal_artifacts
        .iter()
        .find(|artifact| artifact.id == "agent-native-check")
    {
        metrics.push(PublicProofMetric {
            name: "agent_native_check.measured_preflight_passed".to_string(),
            value: serde_json::json!(artifact.status == "pass"),
            target: Some(serde_json::json!(true)),
            status: artifact.status.clone(),
        });
    }
    metrics.push(PublicProofMetric {
        name: "local.default_packet_tokens".to_string(),
        value: serde_json::json!(local_gate.default_packet_tokens),
        target: targets
            .maximum_default_packet_tokens
            .map(|target| serde_json::json!(target)),
        status: metric_status(
            targets
                .maximum_default_packet_tokens
                .is_none_or(|target| local_gate.default_packet_tokens <= target),
        ),
    });
    for evidence in public_benchmarks {
        metrics.push(PublicProofMetric {
            name: format!("public.{}.first_correct_file_rate_at_k", evidence.id),
            value: serde_json::json!(evidence.first_correct_file_rate_at_k),
            target: None,
            status: evidence.status.clone(),
        });
        if let Some(minus_grep) = evidence.minus_grep {
            metrics.push(PublicProofMetric {
                name: format!("public.{}.minus_grep", evidence.id),
                value: serde_json::json!(minus_grep),
                target: None,
                status: metric_status(minus_grep >= 0.0),
            });
        }
    }
    if let Some(best) = &public_result_catalog.best_swe_bench_style {
        metrics.push(PublicProofMetric {
            name: "public_catalog.best_swe_bench_style_first_correct_file_rate_at_k".to_string(),
            value: serde_json::json!(best.first_correct_file_rate_at_k),
            target: Some(serde_json::json!(best.target_first_correct_file_rate_at_k)),
            status: best.status.clone(),
        });
    }
    if let Some(best) = &public_result_catalog.best_natural_language {
        metrics.push(PublicProofMetric {
            name: "public_catalog.best_natural_language_first_correct_file_rate_at_k".to_string(),
            value: serde_json::json!(best.first_correct_file_rate_at_k),
            target: Some(serde_json::json!(best.target_first_correct_file_rate_at_k)),
            status: best.status.clone(),
        });
    }
    for baseline in repo_packer_baselines {
        if let Some(tokens) = baseline.tokens {
            metrics.push(PublicProofMetric {
                name: format!("repo_packer.{}.tokens", baseline.id),
                value: serde_json::json!(tokens),
                target: None,
                status: baseline.status.clone(),
            });
        }
        if let Some(ratio) = baseline.token_ratio_vs_callsieve_packet {
            metrics.push(PublicProofMetric {
                name: format!(
                    "repo_packer.{}.token_ratio_vs_callsieve_packet",
                    baseline.id
                ),
                value: serde_json::json!(ratio),
                target: baseline
                    .minimum_token_ratio_vs_callsieve_packet
                    .map(|target| serde_json::json!(target)),
                status: baseline.status.clone(),
            });
        }
    }
    metrics.push(PublicProofMetric {
        name: "agent_native_search.measured_baseline_count".to_string(),
        value: serde_json::json!(agent_native_search_summary.measured_baseline_count),
        target: Some(serde_json::json!(
            targets
                .minimum_agent_native_measured_baselines
                .unwrap_or(agent_native_search_summary.required_baseline_count)
        )),
        status: metric_status(
            agent_native_search_summary.measured_baseline_count
                >= targets
                    .minimum_agent_native_measured_baselines
                    .unwrap_or(agent_native_search_summary.required_baseline_count),
        ),
    });
    metrics.push(PublicProofMetric {
        name: "agent_native_search.transcript_backed_baseline_count".to_string(),
        value: serde_json::json!(agent_native_search_summary.transcript_backed_baseline_count),
        target: Some(serde_json::json!(
            targets
                .minimum_agent_native_measured_baselines
                .unwrap_or(agent_native_search_summary.required_baseline_count)
        )),
        status: metric_status(
            agent_native_search_summary.transcript_backed_baseline_count
                >= targets
                    .minimum_agent_native_measured_baselines
                    .unwrap_or(agent_native_search_summary.required_baseline_count),
        ),
    });
    metrics.push(PublicProofMetric {
        name: "agent_native_search.distinct_agent_tool_count".to_string(),
        value: serde_json::json!(agent_native_search_summary.distinct_agent_tool_count),
        target: targets
            .minimum_agent_native_distinct_tools
            .map(|target| serde_json::json!(target)),
        status: metric_status(
            targets
                .minimum_agent_native_distinct_tools
                .is_none_or(|minimum| {
                    agent_native_search_summary.distinct_agent_tool_count >= minimum
                }),
        ),
    });
    metrics.push(PublicProofMetric {
        name: "agent_native_search.distinct_repository_count".to_string(),
        value: serde_json::json!(agent_native_search_summary.distinct_repository_count),
        target: targets
            .minimum_agent_native_repositories
            .map(|target| serde_json::json!(target)),
        status: metric_status(
            targets
                .minimum_agent_native_repositories
                .is_none_or(|minimum| {
                    agent_native_search_summary.distinct_repository_count >= minimum
                }),
        ),
    });
    metrics.push(PublicProofMetric {
        name: "agent_native_search.task_language_count".to_string(),
        value: serde_json::json!(agent_native_search_summary.task_language_count),
        target: targets
            .minimum_agent_native_task_languages
            .map(|target| serde_json::json!(target)),
        status: metric_status(
            targets
                .minimum_agent_native_task_languages
                .is_none_or(|minimum| agent_native_search_summary.task_language_count >= minimum),
        ),
    });
    for baseline in agent_native_search_baselines {
        metrics.push(PublicProofMetric {
            name: format!("agent_native_search.{}.transcript_provenance", baseline.id),
            value: serde_json::json!(baseline.transcript_provenance_status == "pass"),
            target: Some(serde_json::json!(true)),
            status: baseline.transcript_provenance_status.clone(),
        });
        if let Some(tasks) = baseline.tasks {
            metrics.push(PublicProofMetric {
                name: format!("agent_native_search.{}.tasks", baseline.id),
                value: serde_json::json!(tasks),
                target: baseline
                    .minimum_tasks
                    .map(|target| serde_json::json!(target)),
                status: baseline.status.clone(),
            });
        }
        if let Some(delta) = baseline.callsieve_minus_agent_native_first_correct_file_rate_at_k {
            metrics.push(PublicProofMetric {
                name: format!(
                    "agent_native_search.{}.callsieve_minus_agent_native_first_correct_file_rate_at_k",
                    baseline.id
                ),
                value: serde_json::json!(delta),
                target: baseline
                    .minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k
                    .map(|target| serde_json::json!(target)),
                status: baseline.status.clone(),
            });
        }
        if let Some(ratio) = baseline.agent_native_context_token_ratio_vs_callsieve {
            metrics.push(PublicProofMetric {
                name: format!(
                    "agent_native_search.{}.context_token_ratio_vs_callsieve",
                    baseline.id
                ),
                value: serde_json::json!(ratio),
                target: baseline
                    .minimum_agent_native_context_token_ratio_vs_callsieve
                    .map(|target| serde_json::json!(target)),
                status: baseline.status.clone(),
            });
        }
    }

    let misses = public_benchmarks
        .iter()
        .map(|evidence| PublicProofMissEvidence {
            report: evidence.id.clone(),
            preferred_arm: evidence.preferred_arm.clone(),
            sampled_misses: evidence.misses.len(),
            sample: evidence.misses.clone(),
        })
        .collect();

    PublicEvidencePack {
        commands: commands.to_vec(),
        fixture_manifests,
        result_reports,
        metrics,
        misses,
        receipt_commands: vec![
            "cargo run -- receipt . --latest --format markdown".to_string(),
            "cargo run -- receipts .".to_string(),
        ],
        terminal_artifacts,
        public_checkouts: public_checkouts.clone(),
        broad_read_guardrail: broad_read_guardrail.clone(),
        context_packet_guardrail: context_packet_guardrail.clone(),
        repo_packer_baselines: repo_packer_baselines.to_vec(),
        agent_native_search_summary: agent_native_search_summary.clone(),
        agent_native_search_baselines: agent_native_search_baselines.to_vec(),
        mcp_surface: mcp_surface.clone(),
        mcp_contract: mcp_contract.clone(),
        agent_setup: agent_setup.clone(),
    }
}

fn metric_status(pass: bool) -> String {
    if pass { "pass" } else { "needs_work" }.to_string()
}

fn public_artifact_evidence(
    inputs: &[PublicProofArtifactInput],
) -> Vec<PublicProofArtifactEvidence> {
    inputs
        .iter()
        .map(|input| {
            let exists = input.path.is_file();
            let (status, validation, source_artifact_hashes) =
                public_artifact_status(input, exists);
            PublicProofArtifactEvidence {
                id: input.id.clone(),
                path: input.path.display().to_string(),
                kind: input.kind.clone(),
                description: input.description.clone(),
                required: input.required,
                exists,
                status,
                validation,
                source_artifact_hashes,
            }
        })
        .collect()
}

fn public_artifact_status(
    input: &PublicProofArtifactInput,
    exists: bool,
) -> (String, String, Vec<String>) {
    if input.id == "mcp-contract" {
        let (status, validation) = public_mcp_contract_artifact_status(input, exists);
        return (status, validation, Vec::new());
    }
    if input.id == "agent-native-template" {
        let (status, validation) = public_agent_native_template_artifact_status(input, exists);
        return (status, validation, Vec::new());
    }
    if input.id == "agent-native-measurement-plan" {
        let (status, validation) =
            public_agent_native_measurement_plan_artifact_status(input, exists);
        return (status, validation, Vec::new());
    }
    if input.id == "agent-native-protocol" {
        let (status, validation) = public_agent_native_protocol_artifact_status(input, exists);
        return (status, validation, Vec::new());
    }
    if input.id == "agent-native-check" {
        return public_agent_native_check_artifact_status(input, exists);
    }
    if exists {
        (
            "pass".to_string(),
            "artifact exists; content validation is not required for this artifact kind"
                .to_string(),
            Vec::new(),
        )
    } else if input.required {
        (
            "needs_work".to_string(),
            "required artifact is missing".to_string(),
            Vec::new(),
        )
    } else {
        (
            "not_provided".to_string(),
            "optional artifact is not present".to_string(),
            Vec::new(),
        )
    }
}

fn public_agent_native_measurement_plan_artifact_status(
    input: &PublicProofArtifactInput,
    exists: bool,
) -> (String, String) {
    if !exists {
        return if input.required {
            (
                "needs_work".to_string(),
                "required agent-native measurement plan artifact is missing".to_string(),
            )
        } else {
            (
                "not_provided".to_string(),
                "optional agent-native measurement plan artifact is not present".to_string(),
            )
        };
    }

    let raw = match fs::read_to_string(&input.path) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to read agent-native measurement plan artifact: {err}"),
            );
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to parse agent-native measurement plan artifact JSON: {err}"),
            );
        }
    };

    let tool = value
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim();
    if tool.is_empty() {
        return (
            "needs_work".to_string(),
            "measurement plan must include tool".to_string(),
        );
    }
    let suite = value
        .get("suite")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim();
    if suite.is_empty() {
        return (
            "needs_work".to_string(),
            "measurement plan must include suite".to_string(),
        );
    }
    let task_count = value
        .get("task_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize;
    let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) else {
        return (
            "needs_work".to_string(),
            "measurement plan must include tasks".to_string(),
        );
    };
    if task_count == 0 || tasks.len() != task_count {
        return (
            "needs_work".to_string(),
            "measurement plan task_count must match a non-empty tasks array".to_string(),
        );
    }
    let constraints = value
        .get("constraints")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if constraints == 0 {
        return (
            "needs_work".to_string(),
            "measurement plan must include constraints".to_string(),
        );
    }
    let Some(post_run_artifacts) = value
        .get("post_run_artifacts")
        .and_then(serde_json::Value::as_object)
    else {
        return (
            "needs_work".to_string(),
            "measurement plan must include post_run_artifacts".to_string(),
        );
    };
    for key in [
        "protocol",
        "task_log",
        "transcript",
        "check",
        "baseline",
        "overlay_manifest",
        "proof",
    ] {
        if post_run_artifacts
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return (
                "needs_work".to_string(),
                format!("measurement plan post_run_artifacts must include {key}"),
            );
        }
    }
    let token_accounting = value
        .get("token_accounting")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .trim();
    if token_accounting.is_empty() {
        return (
            "needs_work".to_string(),
            "measurement plan must include token_accounting".to_string(),
        );
    }

    for task in tasks {
        let task_id = task
            .get("task_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim();
        let prompt = task
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim();
        let command = task.get("command").and_then(serde_json::Value::as_array);
        let raw_transcript = task
            .get("raw_transcript")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .trim();
        if task_id.is_empty()
            || prompt.is_empty()
            || command.is_none_or(Vec::is_empty)
            || raw_transcript.is_empty()
        {
            return (
                "needs_work".to_string(),
                "measurement plan tasks must include task_id, prompt, command, and raw_transcript"
                    .to_string(),
            );
        }
    }

    (
        "pass".to_string(),
        format!("valid {tool} measurement plan for {task_count} {suite} tasks"),
    )
}

fn public_agent_native_template_artifact_status(
    input: &PublicProofArtifactInput,
    exists: bool,
) -> (String, String) {
    if !exists {
        return if input.required {
            (
                "needs_work".to_string(),
                "required agent-native template artifact is missing".to_string(),
            )
        } else {
            (
                "not_provided".to_string(),
                "optional agent-native template artifact is not present".to_string(),
            )
        };
    }

    let raw = match fs::read_to_string(&input.path) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to read agent-native template artifact: {err}"),
            );
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to parse agent-native template artifact JSON: {err}"),
            );
        }
    };

    if value.get("command").and_then(serde_json::Value::as_str) != Some("agent-native-template") {
        return (
            "needs_work".to_string(),
            "command is not agent-native-template".to_string(),
        );
    }
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return (
            "needs_work".to_string(),
            "schema_version must be 1".to_string(),
        );
    }
    if value.get("status").and_then(serde_json::Value::as_str)
        != Some("needs_agent_native_measurement")
    {
        return (
            "needs_work".to_string(),
            "status must remain needs_agent_native_measurement until a real native-search run is recorded".to_string(),
        );
    }
    if value
        .get("locally_measured")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
    {
        return (
            "needs_work".to_string(),
            "template must not be marked locally measured".to_string(),
        );
    }

    let Some(source_tasks) = value
        .get("source_tasks")
        .and_then(serde_json::Value::as_str)
        .filter(|path| !path.trim().is_empty())
    else {
        return (
            "needs_work".to_string(),
            "template must include source_tasks".to_string(),
        );
    };
    let Some(source_tasks_hash) = value
        .get("source_tasks_hash")
        .and_then(serde_json::Value::as_str)
        .filter(|hash| !hash.trim().is_empty())
    else {
        return (
            "needs_work".to_string(),
            "template must include source_tasks_hash".to_string(),
        );
    };
    let source_bytes = match fs::read(source_tasks) {
        Ok(bytes) => bytes,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to read template source_tasks `{source_tasks}`: {err}"),
            );
        }
    };
    let current_source_hash = indexer::stable_content_hash(&source_bytes);
    if current_source_hash != source_tasks_hash {
        return (
            "needs_work".to_string(),
            "source_tasks_hash does not match the current source task file".to_string(),
        );
    }

    let Some(root) = value
        .get("root")
        .and_then(serde_json::Value::as_str)
        .filter(|root| !root.trim().is_empty())
    else {
        return (
            "needs_work".to_string(),
            "template must include root".to_string(),
        );
    };
    let Some(index_fingerprint) = value
        .get("index_fingerprint")
        .and_then(serde_json::Value::as_str)
        .filter(|fingerprint| !fingerprint.trim().is_empty())
    else {
        return (
            "needs_work".to_string(),
            "template must include index_fingerprint".to_string(),
        );
    };
    let pinned_base_commits = value
        .get("pinned_base_commits")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !pinned_base_commits {
        let current_index = match store::json_store::load_index(Path::new(root)) {
            Ok(index) => index,
            Err(err) => {
                return (
                    "needs_work".to_string(),
                    format!("failed to load template root index `{root}`: {err}"),
                );
            }
        };
        let current_index_fingerprint = indexer::index_fingerprint(&current_index);
        if current_index_fingerprint != index_fingerprint {
            return (
                "needs_work".to_string(),
                "index_fingerprint does not match the current local index".to_string(),
            );
        }
    }
    let Some(retrieval_contract_fingerprint) = value
        .get("retrieval_contract_fingerprint")
        .and_then(serde_json::Value::as_str)
        .filter(|fingerprint| !fingerprint.trim().is_empty())
    else {
        return (
            "needs_work".to_string(),
            "template must include retrieval_contract_fingerprint".to_string(),
        );
    };
    let current_retrieval_contract_fingerprint = query::retrieval_contract_fingerprint();
    if current_retrieval_contract_fingerprint != retrieval_contract_fingerprint {
        return (
            "needs_work".to_string(),
            "retrieval_contract_fingerprint does not match the current retrieval contract"
                .to_string(),
        );
    }

    let Some(k) = value.get("k").and_then(serde_json::Value::as_u64) else {
        return (
            "needs_work".to_string(),
            "template must include k".to_string(),
        );
    };
    if k == 0 {
        return (
            "needs_work".to_string(),
            "template k must be greater than zero".to_string(),
        );
    }
    let Some(limit) = value.get("limit").and_then(serde_json::Value::as_u64) else {
        return (
            "needs_work".to_string(),
            "template must include limit".to_string(),
        );
    };
    let Some(snippets_per_file) = value
        .get("snippets_per_file")
        .and_then(serde_json::Value::as_u64)
    else {
        return (
            "needs_work".to_string(),
            "template must include snippets_per_file".to_string(),
        );
    };
    let profile = match value
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
    {
        "skim" => ContextProfileArg::Skim,
        "normal" => ContextProfileArg::Normal,
        "full" => ContextProfileArg::Full,
        _ => {
            return (
                "needs_work".to_string(),
                "template profile must be skim, normal, or full".to_string(),
            );
        }
    };
    let Some(token_budget) = value
        .get("token_budget")
        .and_then(serde_json::Value::as_u64)
    else {
        return (
            "needs_work".to_string(),
            "template must include token_budget".to_string(),
        );
    };

    let Some(tasks) = value.get("tasks").and_then(serde_json::Value::as_array) else {
        return (
            "needs_work".to_string(),
            "template must include a non-empty tasks array".to_string(),
        );
    };
    if tasks.is_empty() {
        return (
            "needs_work".to_string(),
            "template must include at least one task".to_string(),
        );
    }
    for task in tasks {
        let id = task
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let expected_files = task
            .get("expected_files")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let agent_native_files = task
            .get("agent_native_files")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(usize::MAX);
        let callsieve_files = task
            .get("callsieve_files")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let agent_native_context_tokens = task
            .get("agent_native_context_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(u64::MAX);
        let callsieve_packet_tokens = task
            .get("callsieve_packet_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let recording_status = task
            .get("recording_status")
            .and_then(serde_json::Value::as_str);
        if expected_files == 0 {
            return (
                "needs_work".to_string(),
                format!("task `{id}` is missing expected_files"),
            );
        }
        if agent_native_files != 0 || agent_native_context_tokens != 0 {
            return (
                "needs_work".to_string(),
                format!("task `{id}` already contains native-search measurements"),
            );
        }
        if callsieve_files == 0 || callsieve_packet_tokens == 0 {
            return (
                "needs_work".to_string(),
                format!("task `{id}` is missing CallSieve files or packet tokens"),
            );
        }
        if recording_status != Some("needs_agent_native_measurement") {
            return (
                "needs_work".to_string(),
                format!("task `{id}` recording_status must be needs_agent_native_measurement"),
            );
        }
    }
    let recomputed_template = match agent_native_template(
        Path::new(root),
        Path::new(source_tasks),
        AgentNativeTemplateOptions {
            k: k as usize,
            limit: limit as usize,
            snippets_per_file: snippets_per_file as usize,
            include_snippets: snippets_per_file > 0,
            profile,
            token_budget: token_budget as usize,
        },
    ) {
        Ok(output) => output,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to recompute agent-native template: {err}"),
            );
        }
    };
    if recomputed_template.index_fingerprint != index_fingerprint {
        return (
            "needs_work".to_string(),
            "index_fingerprint does not match recomputed agent-native template".to_string(),
        );
    }
    let recomputed_tasks = match serde_json::to_value(&recomputed_template.tasks) {
        Ok(tasks) => tasks,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to serialize recomputed agent-native template tasks: {err}"),
            );
        }
    };
    if value.get("tasks") != Some(&recomputed_tasks) {
        return (
            "needs_work".to_string(),
            "template task payload does not match recomputed CallSieve template".to_string(),
        );
    }

    (
        "pass".to_string(),
        format!(
            "agent-native template is ready for {} measured tasks and matches recomputed source/index/retrieval provenance",
            tasks.len()
        ),
    )
}

fn public_mcp_contract_artifact_status(
    input: &PublicProofArtifactInput,
    exists: bool,
) -> (String, String) {
    if !exists {
        return if input.required {
            (
                "needs_work".to_string(),
                "required MCP contract artifact is missing".to_string(),
            )
        } else {
            (
                "not_provided".to_string(),
                "optional MCP contract artifact is not present".to_string(),
            )
        };
    }

    let raw = match fs::read_to_string(&input.path) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to read MCP contract artifact: {err}"),
            );
        }
    };
    let actual: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(actual) => actual,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to parse MCP contract artifact JSON: {err}"),
            );
        }
    };
    let expected = match serde_json::to_value(mcp_contract_output()) {
        Ok(expected) => expected,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to build live MCP contract JSON: {err}"),
            );
        }
    };
    if actual == expected {
        (
            "pass".to_string(),
            "matches live callsieve mcp-contract output".to_string(),
        )
    } else {
        (
            "needs_work".to_string(),
            "does not match live callsieve mcp-contract output".to_string(),
        )
    }
}

fn public_agent_native_protocol_artifact_status(
    input: &PublicProofArtifactInput,
    exists: bool,
) -> (String, String) {
    if !exists {
        return if input.required {
            (
                "needs_work".to_string(),
                "required agent-native protocol artifact is missing".to_string(),
            )
        } else {
            (
                "not_provided".to_string(),
                "optional agent-native protocol artifact is not present".to_string(),
            )
        };
    }

    let raw = match fs::read_to_string(&input.path) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to read agent-native protocol artifact: {err}"),
            );
        }
    };
    let actual: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(actual) => actual,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to parse agent-native protocol artifact JSON: {err}"),
            );
        }
    };
    let expected = match serde_json::to_value(agent_native_protocol_output()) {
        Ok(expected) => expected,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to build live agent-native protocol JSON: {err}"),
            );
        }
    };
    if actual == expected {
        (
            "pass".to_string(),
            "matches live callsieve agent-native-protocol output".to_string(),
        )
    } else {
        (
            "needs_work".to_string(),
            "does not match live callsieve agent-native-protocol output".to_string(),
        )
    }
}

fn public_agent_native_check_artifact_status(
    input: &PublicProofArtifactInput,
    exists: bool,
) -> (String, String, Vec<String>) {
    if !exists {
        return if input.required {
            (
                "needs_work".to_string(),
                "required agent-native check artifact is missing".to_string(),
                Vec::new(),
            )
        } else {
            (
                "not_provided".to_string(),
                "optional agent-native check artifact is not present".to_string(),
                Vec::new(),
            )
        };
    }

    let raw = match fs::read_to_string(&input.path) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to read agent-native check artifact: {err}"),
                Vec::new(),
            );
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to parse agent-native check artifact JSON: {err}"),
                Vec::new(),
            );
        }
    };

    if value.get("command").and_then(serde_json::Value::as_str) != Some("agent-native-check") {
        return (
            "needs_work".to_string(),
            "command is not agent-native-check".to_string(),
            Vec::new(),
        );
    }
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return (
            "needs_work".to_string(),
            "schema_version must be 1".to_string(),
            Vec::new(),
        );
    }
    if value.get("status").and_then(serde_json::Value::as_str) != Some("pass") {
        return (
            "needs_work".to_string(),
            "agent-native check status must be pass".to_string(),
            Vec::new(),
        );
    }
    if value.get("mode").and_then(serde_json::Value::as_str) != Some("measured") {
        return (
            "needs_work".to_string(),
            "agent-native check mode must be measured".to_string(),
            Vec::new(),
        );
    }

    let task_count = value
        .get("task_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if task_count == 0 {
        return (
            "needs_work".to_string(),
            "agent-native check must cover at least one task".to_string(),
            Vec::new(),
        );
    }
    for field in [
        "tasks_ready_for_mode",
        "tasks_with_expected_files",
        "tasks_with_callsieve_files",
        "tasks_with_callsieve_packet_tokens",
        "tasks_with_agent_native_files",
        "tasks_with_agent_native_context_tokens",
    ] {
        if value.get(field).and_then(serde_json::Value::as_u64) != Some(task_count) {
            return (
                "needs_work".to_string(),
                format!("{field} must equal task_count for measured preflight proof"),
                Vec::new(),
            );
        }
    }
    let issues_empty = value
        .get("issues")
        .and_then(serde_json::Value::as_array)
        .is_some_and(Vec::is_empty);
    if !issues_empty {
        return (
            "needs_work".to_string(),
            "agent-native check artifact must have no issues".to_string(),
            Vec::new(),
        );
    }

    let Some(source_artifacts) = value
        .get("source_artifact_evidence")
        .and_then(serde_json::Value::as_array)
    else {
        return (
            "needs_work".to_string(),
            "agent-native check must include source_artifact_evidence".to_string(),
            Vec::new(),
        );
    };
    if source_artifacts.is_empty() {
        return (
            "needs_work".to_string(),
            "agent-native measured check must include at least one transcript or export source artifact".to_string(),
            Vec::new(),
        );
    }

    let mut task_log_artifacts = 0usize;
    let mut task_log_artifact_path = None;
    let mut transcript_or_export_artifacts = 0usize;
    let mut transcript_or_export_artifact_paths = Vec::new();
    let mut source_artifact_hashes = Vec::new();
    for artifact in source_artifacts {
        let source_path = artifact
            .get("path")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if source_path.trim().is_empty() {
            return (
                "needs_work".to_string(),
                "agent-native check source artifact is missing path".to_string(),
                Vec::new(),
            );
        }
        let role = artifact
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if role != "task_log" && role != "agent_native_transcript_or_export" {
            return (
                "needs_work".to_string(),
                "agent-native check source artifact role must be task_log or agent_native_transcript_or_export".to_string(),
                Vec::new(),
            );
        }
        let declared_bytes = artifact
            .get("bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if declared_bytes == 0 {
            return (
                "needs_work".to_string(),
                "agent-native check source artifact bytes must be greater than 0".to_string(),
                Vec::new(),
            );
        }
        let hash = artifact
            .get("hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !hash.starts_with("fnv1a64:") || hash.len() == "fnv1a64:".len() {
            return (
                "needs_work".to_string(),
                "agent-native check source artifact hash must use fnv1a64".to_string(),
                Vec::new(),
            );
        }

        let source_artifact_path =
            resolve_agent_native_source_artifact_path(&input.path, source_path);
        let bytes = match fs::read(&source_artifact_path) {
            Ok(bytes) => bytes,
            Err(err) => {
                return (
                    "needs_work".to_string(),
                    format!(
                        "failed to read agent-native check source artifact `{source_path}`: {err}"
                    ),
                    Vec::new(),
                );
            }
        };
        if declared_bytes != bytes.len() as u64 {
            return (
                "needs_work".to_string(),
                format!(
                    "agent-native check source artifact `{source_path}` byte count does not match"
                ),
                Vec::new(),
            );
        }
        if indexer::stable_content_hash(&bytes) != hash {
            return (
                "needs_work".to_string(),
                format!("agent-native check source artifact `{source_path}` hash does not match"),
                Vec::new(),
            );
        }
        if role == "task_log" {
            task_log_artifacts += 1;
            task_log_artifact_path = Some(source_artifact_path);
        } else {
            transcript_or_export_artifacts += 1;
            transcript_or_export_artifact_paths.push(source_artifact_path);
        }
        source_artifact_hashes.push(hash.to_string());
    }
    if task_log_artifacts != 1 {
        return (
            "needs_work".to_string(),
            "agent-native check must include exactly one task_log source artifact".to_string(),
            Vec::new(),
        );
    }
    if transcript_or_export_artifacts == 0 {
        return (
            "needs_work".to_string(),
            "agent-native check must include at least one transcript/export source artifact"
                .to_string(),
            Vec::new(),
        );
    }
    let Some(task_log_artifact_path) = task_log_artifact_path else {
        return (
            "needs_work".to_string(),
            "agent-native check must include exactly one task_log source artifact".to_string(),
            Vec::new(),
        );
    };
    let recomputed = match agent_native_check(
        &task_log_artifact_path,
        AgentNativeCheckMode::Measured,
        transcript_or_export_artifact_paths,
    ) {
        Ok(output) => output,
        Err(err) => {
            return (
                "needs_work".to_string(),
                format!("failed to recompute agent-native measured preflight: {err}"),
                Vec::new(),
            );
        }
    };
    if recomputed.status != "pass" {
        let issue = recomputed
            .issues
            .first()
            .map(|issue| format!("{}: {}", issue.field, issue.message))
            .unwrap_or_else(|| recomputed.status.clone());
        return (
            "needs_work".to_string(),
            format!(
                "agent-native check source artifacts do not recompute to a passing measured preflight ({issue})"
            ),
            Vec::new(),
        );
    }
    for (field, recomputed_count) in [
        ("task_count", recomputed.task_count),
        ("tasks_ready_for_mode", recomputed.tasks_ready_for_mode),
        (
            "tasks_with_expected_files",
            recomputed.tasks_with_expected_files,
        ),
        (
            "tasks_with_callsieve_files",
            recomputed.tasks_with_callsieve_files,
        ),
        (
            "tasks_with_callsieve_packet_tokens",
            recomputed.tasks_with_callsieve_packet_tokens,
        ),
        (
            "tasks_with_agent_native_files",
            recomputed.tasks_with_agent_native_files,
        ),
        (
            "tasks_with_agent_native_context_tokens",
            recomputed.tasks_with_agent_native_context_tokens,
        ),
    ] {
        if value.get(field).and_then(serde_json::Value::as_u64) != Some(recomputed_count as u64) {
            return (
                "needs_work".to_string(),
                format!("{field} does not match recomputed measured preflight"),
                Vec::new(),
            );
        }
    }

    (
        "pass".to_string(),
        format!(
            "agent-native measured preflight passed for {task_count} tasks with readable source artifacts"
        ),
        source_artifact_hashes,
    )
}

fn public_miss_triage(path: &Path) -> Result<PublicMissTriage> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("failed to read public miss triage: {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&json)
        .with_context(|| format!("failed to parse public miss triage: {}", path.display()))?;
    let entries = value
        .as_array()
        .context("public miss triage must be a JSON array")?;
    let mut sample = Vec::new();
    let mut reachable = 0usize;
    let mut same_directory_hints = 0usize;
    for entry in entries {
        let entry_reachable = entry
            .get("reachable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if entry_reachable {
            reachable += 1;
        }
        let same_dir = json_string_array(entry.get("same_dir"));
        if !same_dir.is_empty() {
            same_directory_hints += 1;
        }
        if sample.len() < 8 {
            sample.push(PublicMissTriageSample {
                id: entry
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                truth: entry
                    .get("truth")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                reachable: entry_reachable,
                same_dir,
                pool_refs_truth: json_string_array(entry.get("pool_refs_truth")),
                truth_refs_pool: json_string_array(entry.get("truth_refs_pool")),
            });
        }
    }
    Ok(PublicMissTriage {
        path: path.display().to_string(),
        total: entries.len(),
        reachable,
        not_reachable: entries.len().saturating_sub(reachable),
        same_directory_hints,
        sample,
    })
}

fn public_report_arm_rate(value: &serde_json::Value, arm: &str) -> f64 {
    let pointer = match arm {
        "hybrid" => "/aggregate/hybrid_first_correct_file_rate_at_k",
        _ => "/aggregate/lexical_first_correct_file_rate_at_k",
    };
    json_pointer_f64(value, pointer)
        .or_else(|| {
            (arm == "lexical")
                .then(|| json_pointer_f64(value, "/aggregate/first_correct_file_rate_at_k"))
                .flatten()
        })
        .unwrap_or(0.0)
}

fn public_report_manifest_path(value: &serde_json::Value) -> Option<String> {
    json_pointer_string(value, "/manifest/path").or_else(|| json_pointer_string(value, "/manifest"))
}

fn public_report_retrieval_contract_status(value: &serde_json::Value) -> (Option<String>, String) {
    let fingerprint = json_pointer_string(value, "/retrieval_contract_fingerprint");
    let status = match fingerprint.as_deref() {
        Some(actual) if actual == query::retrieval_contract_fingerprint() => "current",
        Some(_) => "stale",
        None => "missing",
    };
    (fingerprint, status.to_string())
}

fn infer_public_report_category(id: &str, value: &serde_json::Value) -> String {
    let id = id.to_ascii_lowercase();
    let manifest_path = public_report_manifest_path(value)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let description = json_pointer_string(value, "/manifest/description")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if id.contains("natural")
        || id.contains("-nl")
        || manifest_path.contains("-nl")
        || description.contains("natural-language")
    {
        "natural_language".to_string()
    } else {
        "swe_bench_style".to_string()
    }
}

fn average_selected_files_for_arm(value: &serde_json::Value, arm: &str) -> f64 {
    let Some(issues) = value.get("issues").and_then(serde_json::Value::as_array) else {
        return 0.0;
    };
    let mut count = 0usize;
    let mut total = 0usize;
    for issue in issues {
        let selected = issue
            .get(arm)
            .and_then(|arm| arm.get("selected_files_count"))
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                (arm == "lexical")
                    .then(|| {
                        issue
                            .get("selected_files_count")
                            .and_then(serde_json::Value::as_u64)
                    })
                    .flatten()
            });
        let Some(selected) = selected else {
            continue;
        };
        count += 1;
        total += selected as usize;
    }
    if count == 0 {
        0.0
    } else {
        total as f64 / count as f64
    }
}

fn public_report_misses(value: &serde_json::Value, arm: &str, limit: usize) -> Vec<String> {
    let Some(issues) = value.get("issues").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    issues
        .iter()
        .filter(|issue| {
            let rate = issue
                .get(arm)
                .and_then(|arm| arm.get("first_correct_file_rate_at_k"))
                .and_then(serde_json::Value::as_f64)
                .or_else(|| {
                    (arm == "lexical")
                        .then(|| {
                            issue
                                .get("first_correct_file_rate_at_k")
                                .and_then(serde_json::Value::as_f64)
                        })
                        .flatten()
                })
                .unwrap_or(0.0);
            rate <= f64::EPSILON
        })
        .filter_map(|issue| issue.get("id").and_then(serde_json::Value::as_str))
        .take(limit)
        .map(str::to_string)
        .collect()
}

fn weighted_average_selected_files(public_benchmarks: &[PublicBenchmarkEvidence]) -> Option<f64> {
    let total_evaluated: usize = public_benchmarks
        .iter()
        .map(|evidence| evidence.evaluated)
        .sum();
    if total_evaluated == 0 {
        return None;
    }
    let weighted_sum: f64 = public_benchmarks
        .iter()
        .map(|evidence| evidence.average_selected_files * evidence.evaluated as f64)
        .sum();
    Some(weighted_sum / total_evaluated as f64)
}

fn json_pointer_usize(value: &serde_json::Value, pointer: &str) -> usize {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize
}

fn json_pointer_f64(value: &serde_json::Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer).and_then(serde_json::Value::as_f64)
}

fn json_pointer_string(value: &serde_json::Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)
        .with_context(|| format!("failed to write JSON artifact {}", path.display()))
}

fn json_metric_usize(
    value: &serde_json::Value,
    explicit_pointer: Option<&str>,
    fallback_pointers: &[&str],
) -> Option<usize> {
    explicit_pointer
        .and_then(|pointer| {
            let pointer = pointer.trim();
            if pointer.is_empty() {
                None
            } else {
                json_metric_value_usize(value.pointer(pointer))
            }
        })
        .or_else(|| {
            fallback_pointers
                .iter()
                .find_map(|pointer| json_metric_value_usize(value.pointer(pointer)))
        })
}

fn json_metric_value_usize(value: Option<&serde_json::Value>) -> Option<usize> {
    match value? {
        serde_json::Value::Number(number) => number
            .as_u64()
            .map(|value| value as usize)
            .or_else(|| number.as_f64().map(|value| value.max(0.0).round() as usize)),
        serde_json::Value::String(value) => parse_metric_usize(value),
        _ => None,
    }
}

fn parse_metric_usize(value: &str) -> Option<usize> {
    let mut seen_digit = false;
    let mut normalized = String::new();
    for ch in value.trim().chars() {
        if ch.is_ascii_digit() {
            seen_digit = true;
            normalized.push(ch);
        } else if seen_digit && matches!(ch, ',' | '_' | ' ') {
            continue;
        } else if seen_digit {
            break;
        }
    }
    if normalized.is_empty() {
        None
    } else {
        normalized.parse().ok()
    }
}

fn default_public_proof_commands(manifest_path: &Path) -> Vec<String> {
    let mut commands = vec![
        "cargo run -- index .".to_string(),
        "cargo run -- competitive-report benchmarks/competitive-response-manifest.example.json"
            .to_string(),
        format!(
            "cargo run -- public-proof-report {}",
            manifest_path.display()
        ),
        "cargo run -- mcp-contract --out benchmarks/public/results/mcp-contract.json".to_string(),
        "cargo run -- agent-native-protocol --out benchmarks/public/results/agent-native-protocol.json".to_string(),
        "cargo run -- agent-native-template benchmarks/public/repos/psf/requests benchmarks/public/manifest.json --k 5 --out benchmarks/public/results/agent-native-requests-template.json".to_string(),
    ];
    commands.extend(public_checkout_commands(&default_public_repos_dir()));
    commands
}

fn competitive_callsieve_evidence(
    value: &serde_json::Value,
    trace_policy: Option<&serde_json::Value>,
) -> CompetitiveCallsieveEvidence {
    let tasks = json_summary_usize(value, "tasks");
    let callsieve_tokens = json_summary_usize(value, "callsieve_context_payload_tokens_estimate");
    let default_packet_tokens = if tasks == 0 {
        0
    } else {
        callsieve_tokens.div_ceil(tasks)
    };
    let strict_trace_policy_sessions =
        trace_policy.map_or(0, |value| json_value_usize(value, "sessions"));
    let strict_trace_policy_violations =
        trace_policy.map_or(0, |value| json_value_usize(value, "violations"));
    let context_first_compliant = strict_trace_policy_sessions > 0
        && strict_trace_policy_violations == 0
        && trace_policy
            .and_then(|value| value.get("context_first_compliant"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    let trace_summary_proof_available = competitive_trace_summary_proof_available(value);
    let agent_coverage = competitive_agent_coverage();
    let agent_coverage_count = agent_coverage.len();
    CompetitiveCallsieveEvidence {
        evidence_tier: "local_benchmark_report",
        locally_measured: true,
        first_correct_file_rate_at_k: json_summary_f64(value, "first_correct_file_rate_at_k"),
        expected_file_recall: json_summary_f64(value, "expected_file_recall"),
        default_packet_tokens,
        baseline_context_payload_tokens: json_summary_usize(
            value,
            "baseline_context_payload_tokens_estimate",
        ),
        callsieve_context_payload_tokens: callsieve_tokens,
        context_payload_reduction_percent: value
            .pointer("/summary/context_payload_reduction/context_payload_reduction_percent")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        packet_quality: public_context_packet_quality_evidence(value),
        avoided_grep_commands: json_summary_usize(value, "total_avoided_grep_commands"),
        avoided_file_reads: json_summary_usize(value, "total_avoided_file_reads"),
        first_query_p50_ms: None,
        first_query_p95_ms: None,
        first_query_samples: None,
        retrieval_model_tokens: 0,
        per_query_retrieval_cost_usd: 0.0,
        default_code_upload_required: false,
        explainability: "deterministic ranking reasons plus compact context.sel evidence",
        agent_coverage,
        agent_coverage_count,
        missing_required_agents: Vec::new(),
        strict_trace_policy_sessions,
        strict_trace_policy_violations,
        context_first_compliant,
        trace_or_receipt_proof_available: trace_summary_proof_available && context_first_compliant,
    }
}

fn public_context_packet_quality_evidence(
    value: &serde_json::Value,
) -> PublicContextPacketQualityEvidence {
    PublicContextPacketQualityEvidence {
        tasks: json_pointer_usize(value, "/summary/packet_quality/tasks"),
        selected_files: json_pointer_usize(value, "/summary/packet_quality/selected_files"),
        files_with_symbols: json_pointer_usize(value, "/summary/packet_quality/files_with_symbols"),
        selected_symbols: json_pointer_usize(value, "/summary/packet_quality/selected_symbols"),
        files_with_snippets: json_pointer_usize(
            value,
            "/summary/packet_quality/files_with_snippets",
        ),
        snippets: json_pointer_usize(value, "/summary/packet_quality/snippets"),
        files_with_related_tests: json_pointer_usize(
            value,
            "/summary/packet_quality/files_with_related_tests",
        ),
        related_tests: json_pointer_usize(value, "/summary/packet_quality/related_tests"),
        files_with_blast_radius: json_pointer_usize(
            value,
            "/summary/packet_quality/files_with_blast_radius",
        ),
        blast_radius_hints: json_pointer_usize(value, "/summary/packet_quality/blast_radius_hints"),
        files_with_call_graph_hints: json_pointer_usize(
            value,
            "/summary/packet_quality/files_with_call_graph_hints",
        ),
        call_graph_hints: json_pointer_usize(value, "/summary/packet_quality/call_graph_hints"),
        files_with_non_unknown_risk: json_pointer_usize(
            value,
            "/summary/packet_quality/files_with_non_unknown_risk",
        ),
        files_with_selection_reasons: json_pointer_usize(
            value,
            "/summary/packet_quality/files_with_selection_reasons",
        ),
        selection_reasons: json_pointer_usize(value, "/summary/packet_quality/selection_reasons"),
        files_with_selection_confidence: json_pointer_usize(
            value,
            "/summary/packet_quality/files_with_selection_confidence",
        ),
        selection_signals: json_pointer_usize(value, "/summary/packet_quality/selection_signals"),
        next_file_hints: json_pointer_usize(value, "/summary/packet_quality/next_file_hints"),
        focus_targets: json_pointer_usize(value, "/summary/packet_quality/focus_targets"),
        relationship_followup_targets: json_pointer_usize(
            value,
            "/summary/packet_quality/relationship_followup_targets",
        ),
        test_followup_targets: json_pointer_usize(
            value,
            "/summary/packet_quality/test_followup_targets",
        ),
    }
}

fn competitive_trace_summary_proof_available(value: &serde_json::Value) -> bool {
    let trace = value
        .pointer("/summary/observed_session")
        .filter(|value| !value.is_null())
        .or_else(|| {
            value
                .pointer("/summary/session_trace")
                .filter(|value| !value.is_null())
        });
    let Some(trace) = trace else {
        return false;
    };

    json_value_usize(trace, "sessions") > 0
        && json_value_isize(trace, "token_savings") > 0
        && (json_value_usize(trace, "avoided_grep_commands") > 0
            || json_value_usize(trace, "avoided_file_reads") > 0)
        && json_value_usize(trace, "files_still_missed") == 0
        && json_value_usize(trace, "critical_files_still_missed") == 0
}

fn json_summary_f64(value: &serde_json::Value, key: &str) -> f64 {
    value
        .get("summary")
        .and_then(|summary| summary.get(key))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}

fn json_summary_usize(value: &serde_json::Value, key: &str) -> usize {
    value
        .get("summary")
        .and_then(|summary| summary.get(key))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize
}

fn json_value_usize(value: &serde_json::Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as usize
}

fn json_value_isize(value: &serde_json::Value, key: &str) -> isize {
    value
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0) as isize
}

fn callsieve_comparison_row(
    callsieve: &CompetitiveCallsieveEvidence,
    natural_language: Option<&CompetitiveCallsieveEvidence>,
) -> CompetitiveComparisonRow {
    CompetitiveComparisonRow {
        name: "CallSieve".to_string(),
        category: "local-first agent context layer".to_string(),
        evidence_kind: "locally_measured",
        source: "competitive-report benchmark manifest".to_string(),
        install_friction: "local CLI, local index, optional MCP setup".to_string(),
        first_query_time: match (
            callsieve.first_query_p50_ms,
            callsieve.first_query_p95_ms,
            callsieve.first_query_samples,
        ) {
            (Some(p50), Some(p95), Some(samples)) => {
                format!("measured local load+context p50 {p50}ms, p95 {p95}ms over {samples} samples")
            }
            _ => "not measured".to_string(),
        },
        code_upload_requirement: "none by default".to_string(),
        retrieval_model_token_cost: callsieve.retrieval_model_tokens.to_string(),
        per_query_retrieval_cost: "$0.00 local retrieval".to_string(),
        packet_compactness: format!(
            "{} average packet tokens, {} total packet tokens",
            callsieve.default_packet_tokens, callsieve.callsieve_context_payload_tokens
        ),
        first_correct_file_rate_at_k: format_rate(callsieve.first_correct_file_rate_at_k),
        natural_language_recall: natural_language
            .map(|evidence| format_rate(evidence.first_correct_file_rate_at_k))
            .unwrap_or_else(|| "not measured".to_string()),
        explainability: callsieve.explainability.to_string(),
        agent_coverage: competitive_agent_coverage().join(", "),
        trace_or_receipt_proof: if callsieve.trace_or_receipt_proof_available {
            format!(
                "strict trace policy pass over {} session(s), with savings evidence",
                callsieve.strict_trace_policy_sessions
            )
        } else if callsieve.strict_trace_policy_sessions > 0 {
            format!(
                "strict trace policy has {} violation(s); proof gate not satisfied",
                callsieve.strict_trace_policy_violations
            )
        } else {
            "not provided in this report".to_string()
        },
        where_callsieve_should_win:
            "first-mile local selection before grep/read loops, with receipts and agent-neutral setup"
                .to_string(),
    }
}

fn competitor_comparison_row(input: CompetitiveCompetitorInput) -> CompetitiveComparisonRow {
    CompetitiveComparisonRow {
        name: input.name,
        category: input.category,
        evidence_kind: "source_claim",
        source: input.source,
        install_friction: input.install_friction,
        first_query_time: input.first_query_time,
        code_upload_requirement: input.code_upload_requirement,
        retrieval_model_token_cost: input.retrieval_model_token_cost,
        per_query_retrieval_cost: input.per_query_retrieval_cost,
        packet_compactness: input.packet_compactness,
        first_correct_file_rate_at_k: "not locally measured".to_string(),
        natural_language_recall: "not locally measured".to_string(),
        explainability: input.explainability,
        agent_coverage: input.agent_coverage,
        trace_or_receipt_proof: input.trace_or_receipt_proof,
        where_callsieve_should_win: input.where_callsieve_should_win,
    }
}

fn competitive_agent_coverage() -> Vec<&'static str> {
    vec![
        "Codex",
        "Claude Code",
        "GitHub Copilot",
        "OpenCode",
        "Antigravity",
        "Cursor",
        "VS Code",
        "Windsurf",
        "Continue",
        "Zed",
        "Junie",
        "JetBrains",
        "Amp",
        "Goose",
        "Warp",
        "Cline",
        "Zoo",
        "Roo",
        "generic MCP",
    ]
}

fn missing_required_agents(
    covered_agents: &[&'static str],
    required_agents: &[String],
) -> Vec<String> {
    let covered: BTreeSet<String> = covered_agents
        .iter()
        .map(|agent| normalized_agent_name(agent))
        .collect();
    required_agents
        .iter()
        .filter(|agent| !covered.contains(&normalized_agent_name(agent)))
        .cloned()
        .collect()
}

fn normalized_agent_name(agent: &str) -> String {
    agent
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn format_rate(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn proof_rehearsal_context_payload_reduction() -> Result<serde_json::Value> {
    let output = benchmark_report_file(
        Path::new(REHEARSAL_EXTERNAL_MANIFEST),
        REHEARSAL_REPORT_LIMIT,
        REHEARSAL_SNIPPETS_PER_FILE,
        true,
    )?;
    let value = serde_json::to_value(output)?;
    value
        .get("summary")
        .and_then(|summary| summary.get("context_payload_reduction"))
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!("benchmark report did not include context_payload_reduction")
        })
}

fn trace_replay_file(
    root: &Path,
    tasks: &Path,
    output: &Path,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<query::TraceReplayOutput> {
    let index = store::json_store::load_index(root)?;
    let tasks_json = fs::read_to_string(tasks)
        .with_context(|| format!("failed to read benchmark suite: {}", tasks.display()))?;
    let suite: query::BenchmarkSuiteInput = serde_json::from_str(&tasks_json)
        .with_context(|| format!("failed to parse benchmark suite: {}", tasks.display()))?;
    let replay = query::trace_replay(
        root,
        &index,
        suite,
        limit,
        snippets_per_file,
        include_snippets,
    )?;
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output directory: {}", parent.display()))?;
    }
    fs::write(output, serde_json::to_vec_pretty(&replay)?)
        .with_context(|| format!("failed to write trace replay: {}", output.display()))?;
    Ok(replay)
}

fn step_summary(value: &serde_json::Value) -> serde_json::Value {
    let mut summary = value
        .get("summary")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(object) = summary.as_object_mut() {
        if let Some(status) = value.get("status") {
            object.insert("status".to_string(), status.clone());
        }
        if let Some(command) = value.get("command") {
            object.insert("command".to_string(), command.clone());
        }
    }
    summary
}

fn assert_status_pass(value: &serde_json::Value, label: &str) -> Result<()> {
    if value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        != "pass"
    {
        anyhow::bail!("{label} did not pass")
    }
    Ok(())
}

fn assert_retrieval_rehearsal(value: &serde_json::Value) -> Result<()> {
    assert_status_pass(value, "eval-retrieval")?;
    let missed = json_path_usize(value, &["summary", "missed_critical_files"]);
    if missed != 0 {
        anyhow::bail!("retrieval rehearsal missed {missed} critical file(s)")
    }
    Ok(())
}

fn assert_external_report_complete(value: &serde_json::Value) -> Result<()> {
    let expected = json_path_usize(value, &["summary", "expected_files"]);
    let found = json_path_usize(value, &["summary", "expected_files_found"]);
    let missed = json_path_usize(value, &["summary", "missed_expected_files"]);
    if expected != 28 || found != 28 || missed != 0 {
        anyhow::bail!(
            "external benchmark report expected 28/28 files, got {found}/{expected} with {missed} missed"
        )
    }
    Ok(())
}

fn json_path_usize(value: &serde_json::Value, path: &[&str]) -> usize {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return 0;
        };
        current = next;
    }
    current
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}

fn setup_observed_codex_oss_50(
    manifest: &Path,
    bootstrap_repos: bool,
    force: bool,
    skip_repo_check: bool,
) -> Result<ObservedSetupOutput> {
    setup_observed_oss_50(
        manifest,
        bootstrap_repos,
        force,
        skip_repo_check,
        AgentClient::Codex,
        "gpt-5-codex",
        "setup-observed-codex-oss-50",
        "real_codex_chatgpt_developer_sessions",
        true,
    )
}

fn setup_observed_claude_oss_50(
    manifest: &Path,
    model: &str,
    bootstrap_repos: bool,
    force: bool,
    skip_repo_check: bool,
) -> Result<ObservedSetupOutput> {
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("model is required")
    }
    setup_observed_oss_50(
        manifest,
        bootstrap_repos,
        force,
        skip_repo_check,
        AgentClient::Claude,
        model,
        "setup-observed-claude-oss-50",
        "real_claude_code_developer_sessions",
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn setup_observed_oss_50(
    manifest: &Path,
    bootstrap_repos: bool,
    force: bool,
    skip_repo_check: bool,
    client: AgentClient,
    model: &str,
    command_name: &'static str,
    collection: &'static str,
    require_codex_bootstrap: bool,
) -> Result<ObservedSetupOutput> {
    for fixture in EXTERNAL_BENCHMARK_FIXTURES {
        if !Path::new(fixture.suite).is_file() {
            anyhow::bail!("missing task suite: {}", fixture.suite);
        }
        if !skip_repo_check && !Path::new(fixture.repo).is_dir() {
            anyhow::bail!("missing fixture repo: {}", fixture.repo);
        }
    }

    let mut rows = observed_codex_oss_50_rows()?;
    let agent_slug = agent_client_name(client);
    if agent_slug != "codex" {
        for row in &mut rows {
            row.id = row.id.replace("-codex-", &format!("-{agent_slug}-"));
        }
    }
    let artifact_root = pilot_artifact_root(manifest);
    for row in &rows {
        let task_dir = artifact_root.join("tasks").join(&row.id);
        if task_dir.exists() {
            anyhow::bail!(
                "refusing to reuse existing task artifact directory: {}",
                task_dir.display()
            );
        }
    }

    if manifest.exists() {
        if !force {
            anyhow::bail!(
                "manifest already exists: {}. Pass --force only for an uncollected local setup.",
                manifest.display()
            );
        }
        fs::remove_file(manifest)
            .with_context(|| format!("failed to remove {}", manifest.display()))?;
    }

    pilot_init(manifest, 50)?;
    apply_observed_oss_50_protocol(manifest, collection, require_codex_bootstrap)?;

    for row in &rows {
        pilot_task_add(
            manifest,
            Path::new(&row.repo),
            &row.task,
            Some(row.id.clone()),
            row.expected_files.clone(),
            row.critical_files.clone(),
            true,
            client,
            model,
            Some(PathBuf::from(&row.suite)),
            Some(row.id.clone()),
            "code_change".to_string(),
            "unknown".to_string(),
            "paired_observed".to_string(),
            "transcript_context_tokens".to_string(),
        )?;
    }

    let repos: Vec<String> = rows
        .iter()
        .map(|row| row.repo.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut bootstrap_outputs = Vec::new();
    if bootstrap_repos {
        for repo in &repos {
            let bootstrap = bootstrap(Path::new(repo), client, true, true, true)?;
            bootstrap_outputs.push(serde_json::to_value(bootstrap)?);
            let doctor = doctor(Path::new(repo), client, false, true)?;
            bootstrap_outputs.push(serde_json::to_value(doctor)?);
        }
    }

    let manifest_value = read_pilot_manifest(manifest)?;
    if manifest_value.tasks.len() != 50 {
        anyhow::bail!(
            "generated manifest has {} tasks; expected 50",
            manifest_value.tasks.len()
        );
    }

    Ok(ObservedSetupOutput {
        command: command_name,
        status: "ready_for_observed_collection".to_string(),
        manifest: manifest.display().to_string(),
        task_count: manifest_value.tasks.len(),
        target_sessions: manifest_value.target_sessions,
        repos,
        bootstrap: bootstrap_outputs,
        next_qa: format!("callsieve pilot-qa {}", manifest.display()),
        final_proof: format!(
            "callsieve pilot-finalize {} --out benchmarks/evidence/proof.local.json --limit {}",
            manifest.display(),
            REHEARSAL_REPORT_LIMIT
        ),
    })
}

fn apply_observed_oss_50_protocol(
    manifest_path: &Path,
    collection: &'static str,
    require_codex_bootstrap: bool,
) -> Result<()> {
    let mut manifest = read_pilot_manifest(manifest_path)?;
    manifest.protocol = PilotEvidenceProtocol {
        evidence_standard: "observed_session_only".to_string(),
        collection: collection.to_string(),
        pairing: "paired_baseline_and_callsieve_phases".to_string(),
        token_accounting: "transcript_context_tokens".to_string(),
        controlled_replay_policy: "reported_separately_never_counted_as_observed".to_string(),
        planned_task_buffer_ratio: 1.2,
        minimum_planned_tasks: 50,
        qa_batch_size: 10,
    };
    manifest.thresholds = serde_json::json!({
        "minimum_recall": 1.0,
        "minimum_token_reduction_percent": 0.0,
        "minimum_observed_sessions": 50,
        "minimum_observed_token_reduction_percent": 50.0,
        "minimum_external_repos": 6,
        "minimum_planned_tasks": 50,
        "maximum_controlled_replay_ratio": 0.0,
        "maximum_trace_violations": 0,
        "maximum_critical_misses": 0,
        "require_fresh_index": true,
        "require_lsp_where_available": true,
        "require_codex_bootstrap": require_codex_bootstrap,
        "require_transcript_token_accounting": true
    });
    write_pilot_manifest(manifest_path, &manifest)
}

fn observed_codex_oss_50_rows() -> Result<Vec<ObservedCodexTaskRow>> {
    let mut base_rows = Vec::new();
    for fixture in EXTERNAL_BENCHMARK_FIXTURES {
        let suite_json = fs::read_to_string(fixture.suite)
            .with_context(|| format!("failed to read {}", fixture.suite))?;
        let suite: serde_json::Value = serde_json::from_str(&suite_json)
            .with_context(|| format!("failed to parse {}", fixture.suite))?;
        let tasks = suite
            .get("tasks")
            .and_then(serde_json::Value::as_array)
            .with_context(|| format!("suite has no tasks array: {}", fixture.suite))?;
        for task in tasks {
            let id = task
                .get("id")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("suite task is missing id: {}", fixture.suite))?;
            let task_text = task
                .get("task")
                .and_then(serde_json::Value::as_str)
                .with_context(|| format!("suite task is missing task text: {}", fixture.suite))?;
            let expected_files = json_string_array(task.get("expected_files"));
            if expected_files.is_empty() {
                anyhow::bail!("suite task is missing expected_files: {id}");
            }
            let critical_files = if task.get("critical_files").is_some() {
                json_string_array(task.get("critical_files"))
            } else {
                expected_files.clone()
            };
            base_rows.push(ObservedCodexTaskRow {
                id: id.to_string(),
                repo: fixture.repo.to_string(),
                suite: fixture.suite.to_string(),
                task: task_text.to_string(),
                expected_files,
                critical_files,
            });
        }
    }

    if base_rows.len() != 12 {
        anyhow::bail!(
            "expected 12 base tasks across external suites, found {}",
            base_rows.len()
        );
    }

    let mut rows = Vec::new();
    for round in 1..=4 {
        for base in &base_rows {
            let mut row = base.clone();
            row.id = format!("{}-codex-r{round:02}", base.id);
            rows.push(row);
        }
    }
    for extra_base_id in ["ripgrep-ignore-walk", "httpx-timeouts-client"] {
        let base = base_rows
            .iter()
            .find(|row| row.id == extra_base_id)
            .with_context(|| format!("missing extra milestone task: {extra_base_id}"))?;
        let mut row = base.clone();
        row.id = format!("{}-codex-r05", base.id);
        rows.push(row);
    }

    let ids = rows
        .iter()
        .map(|row| row.id.clone())
        .collect::<BTreeSet<_>>();
    if rows.len() != 50 || ids.len() != 50 {
        anyhow::bail!(
            "expected 50 unique milestone task rows, found {} rows and {} unique ids",
            rows.len(),
            ids.len()
        );
    }
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn record_codex_observed_session(
    manifest: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    event_command: &str,
    tokens: usize,
    files_read: Vec<String>,
    context_selected_files: Vec<String>,
    dry_run: bool,
) -> Result<RecordObservedSessionOutput> {
    record_observed_session(
        "record-codex-observed-session",
        manifest,
        Some(AgentClient::Codex),
        Some("gpt-5-codex"),
        task_id,
        mode,
        event_command,
        Some(tokens),
        None,
        files_read,
        context_selected_files,
        dry_run,
    )
}

#[allow(clippy::too_many_arguments)]
fn collect_claude_observed_session(
    manifest: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    model: &str,
    max_budget_usd: &str,
    artifact: Option<&Path>,
    context_limit: usize,
    snippets_per_file: usize,
    allowed_tools: Vec<String>,
    dry_run: bool,
) -> Result<CollectClaudeObservedSessionOutput> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        anyhow::bail!("task_id is required")
    }
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("model is required")
    }
    let max_budget_usd = max_budget_usd.trim();
    if max_budget_usd.is_empty() {
        anyhow::bail!("max_budget_usd is required")
    }
    let task = pilot_task_from_manifest(manifest, task_id)?;
    if task.status == "rejected" {
        anyhow::bail!("pilot task is rejected and cannot be collected: {task_id}");
    }
    let repo = Path::new(&task.repo);
    let artifact = artifact.map(Path::to_path_buf).unwrap_or_else(|| {
        Path::new(".callsieve").join(format!(
            "observed-{}-{}.ndjson",
            safe_pilot_label(task_id),
            pilot_session_mode_name(mode)
        ))
    });
    let allowed_tools = normalize_claude_allowed_tools(allowed_tools);
    let prompt_plan =
        claude_observed_prompt_plan(repo, &task, mode, context_limit, snippets_per_file)?;
    let prompt = prompt_plan.prompt.clone();
    let prompt_tokens_estimate = prompt.len().div_ceil(4);
    let command_summary = claude_observed_command_summary(
        repo,
        &task.task,
        mode,
        model,
        max_budget_usd,
        &allowed_tools,
    );

    let record = if dry_run {
        None
    } else {
        if let Some(parent) = artifact
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut child = ProcessCommand::new("claude")
            .current_dir(repo)
            .arg("-p")
            .arg("--input-format")
            .arg("text")
            .arg("--model")
            .arg(model)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--no-session-persistence")
            .arg("--max-budget-usd")
            .arg(max_budget_usd)
            .arg("--permission-mode")
            .arg("acceptEdits")
            .arg("--tools")
            .arg(allowed_tools.join(","))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run Claude Code in {}", repo.display()))?;
        {
            let mut stdin = child.stdin.take().context("failed to open Claude stdin")?;
            stdin
                .write_all(prompt.as_bytes())
                .context("failed to write prompt to Claude stdin")?;
        }
        let output = child
            .wait_with_output()
            .context("failed to wait for Claude Code")?;
        fs::write(&artifact, &output.stdout)
            .with_context(|| format!("failed to write {}", artifact.display()))?;
        let stderr_path = artifact.with_extension("stderr.txt");
        if !output.stderr.is_empty() {
            fs::write(&stderr_path, &output.stderr)
                .with_context(|| format!("failed to write {}", stderr_path.display()))?;
        }
        if !output.status.success() {
            anyhow::bail!(
                "Claude Code exited with status {:?}. Stream saved to {} and stderr saved to {}.",
                output.status.code(),
                artifact.display(),
                stderr_path.display()
            );
        }
        Some(record_observed_session(
            "record-observed-session",
            manifest,
            Some(AgentClient::Claude),
            Some(model),
            task_id,
            mode,
            &command_summary,
            None,
            Some(&artifact),
            Vec::new(),
            prompt_plan.context_selected_files.clone(),
            false,
        )?)
    };

    Ok(CollectClaudeObservedSessionOutput {
        command: "collect-claude-observed-session",
        status: if dry_run { "dry_run" } else { "recorded" }.to_string(),
        manifest: manifest.display().to_string(),
        task_id: task_id.to_string(),
        mode,
        repo: task.repo,
        model: model.to_string(),
        artifact: artifact.display().to_string(),
        max_budget_usd: max_budget_usd.to_string(),
        context_limit,
        snippets_per_file,
        allowed_tools,
        prompt_tokens_estimate,
        context_selected_files: prompt_plan.context_selected_files,
        claude_command: command_summary,
        record,
    })
}

fn proof_sprint_init(
    manifest: &Path,
    client: AgentClient,
    sessions: usize,
    model: &str,
    force: bool,
    skip_repo_check: bool,
) -> Result<ProofSprintInitOutput> {
    if !matches!(client, AgentClient::Claude) {
        anyhow::bail!("proof-sprint currently supports --client claude")
    }
    if !matches!(sessions, 10 | 50) {
        anyhow::bail!("proof-sprint --sessions must be 10 or 50")
    }

    setup_observed_claude_oss_50(manifest, model, false, force, skip_repo_check)?;
    configure_proof_sprint_manifest(manifest, sessions)?;
    let manifest_value = read_pilot_manifest(manifest)?;
    let repos = proof_sprint_repos(&manifest_value);
    let next_task = manifest_value
        .tasks
        .first()
        .map(|task| task.id.as_str())
        .unwrap_or("<task-id>");

    Ok(ProofSprintInitOutput {
        command: "proof-sprint init",
        status: "ready_for_observed_collection".to_string(),
        manifest: manifest.display().to_string(),
        client: "claude".to_string(),
        model: model.trim().to_string(),
        task_count: manifest_value.tasks.len(),
        target_sessions: manifest_value.target_sessions,
        repos,
        next_status: format!("callsieve proof-sprint status {}", manifest.display()),
        next_collect: format!(
            "callsieve proof-sprint collect {} --task-id {} --mode baseline",
            manifest.display(),
            next_task
        ),
        final_proof: format!(
            "callsieve proof-sprint finalize {} --out benchmarks/evidence/proof.local.json --limit {}",
            manifest.display(),
            REHEARSAL_REPORT_LIMIT
        ),
    })
}

fn configure_proof_sprint_manifest(manifest_path: &Path, sessions: usize) -> Result<()> {
    let mut manifest = read_pilot_manifest(manifest_path)?;
    manifest.target_sessions = sessions;
    manifest.protocol.minimum_planned_tasks = manifest.tasks.len();
    if !manifest.thresholds.is_object() {
        manifest.thresholds = serde_json::json!({});
    }
    let thresholds = manifest
        .thresholds
        .as_object_mut()
        .context("proof-sprint thresholds must be a JSON object")?;
    thresholds.insert(
        "minimum_observed_sessions".to_string(),
        serde_json::json!(sessions),
    );
    thresholds.insert(
        "minimum_planned_tasks".to_string(),
        serde_json::json!(manifest.tasks.len()),
    );
    thresholds.insert(
        "minimum_observed_token_reduction_percent".to_string(),
        serde_json::json!(50.0),
    );
    thresholds.insert("maximum_critical_misses".to_string(), serde_json::json!(0));
    thresholds.insert("maximum_trace_violations".to_string(), serde_json::json!(0));
    thresholds.insert(
        "require_transcript_token_accounting".to_string(),
        serde_json::json!(true),
    );
    write_pilot_manifest(manifest_path, &manifest)
}

fn proof_sprint_status(manifest_path: &Path) -> Result<ProofSprintStatusOutput> {
    let manifest = read_pilot_manifest(manifest_path)?;
    let qa = pilot_qa(manifest_path)?;
    let mut missing_baseline_phases = Vec::new();
    let mut missing_callsieve_phases = Vec::new();
    let mut baseline_tokens = 0usize;
    let mut callsieve_tokens = 0usize;
    let mut complete_trace_count = 0usize;
    let mut transcript_accounted_traces = 0usize;
    let mut critical_misses = 0usize;
    let mut strict_trace_violations = 0usize;

    for task in manifest
        .tasks
        .iter()
        .filter(|task| task.status != "rejected")
    {
        let baseline_exists = Path::new(&task.baseline_trace_path).is_file();
        let callsieve_exists = Path::new(&task.callsieve_trace_path).is_file();
        if !baseline_exists {
            missing_baseline_phases.push(task.id.clone());
        }
        if !callsieve_exists {
            missing_callsieve_phases.push(task.id.clone());
        }

        let trace_path = Path::new(&task.trace_path);
        if trace_path.is_file() {
            let trace_json = fs::read_to_string(trace_path)
                .with_context(|| format!("failed to read trace: {}", trace_path.display()))?;
            let trace_value: serde_json::Value = serde_json::from_str(&trace_json)
                .with_context(|| format!("failed to parse trace: {}", trace_path.display()))?;
            let summary = query::trace_summary_from_str(&trace_json)?;
            let summary_value = serde_json::to_value(&summary)?;
            baseline_tokens += summary_number(&summary_value, "baseline_tokens");
            callsieve_tokens += summary_number(&summary_value, "callsieve_tokens");
            critical_misses += summary_number(&summary_value, "critical_files_still_missed");
            complete_trace_count += 1;
            if trace_token_accounting_source(&trace_value) == "transcript_context_tokens" {
                transcript_accounted_traces += 1;
            }
        }

        if callsieve_exists {
            let callsieve_trace_json = fs::read_to_string(&task.callsieve_trace_path)
                .with_context(|| format!("failed to read trace: {}", task.callsieve_trace_path))?;
            let policy = query::trace_check_from_str_with_options(&callsieve_trace_json, true)?;
            let policy_value = serde_json::to_value(policy)?;
            strict_trace_violations += summary_number(&policy_value, "violations");
        }
    }

    let observed_token_reduction_percent = if baseline_tokens > 0 {
        Some(((baseline_tokens as f64 - callsieve_tokens as f64) / baseline_tokens as f64) * 100.0)
    } else {
        None
    };
    let transcript_accounting_coverage_percent = if complete_trace_count == 0 {
        0.0
    } else {
        (transcript_accounted_traces as f64 / complete_trace_count as f64) * 100.0
    };
    let next_command = proof_sprint_next_command(manifest_path, &manifest, &qa);
    let status = if qa.status == "pass" {
        "ready_to_finalize"
    } else if qa.observed_sessions >= manifest.target_sessions {
        "qa_blocked"
    } else {
        "collecting"
    };

    Ok(ProofSprintStatusOutput {
        command: "proof-sprint status",
        manifest: manifest_path.display().to_string(),
        status: status.to_string(),
        client: proof_sprint_client(&manifest),
        target_sessions: manifest.target_sessions,
        planned_tasks: manifest.tasks.len(),
        paired_sessions_complete: qa.observed_sessions,
        rejected_sessions: qa.rejected_sessions,
        missing_baseline_phases,
        missing_callsieve_phases,
        observed_token_reduction_percent,
        critical_misses,
        strict_trace_violations,
        transcript_accounting_coverage_percent,
        qa_status: qa.status,
        qa_failures: qa.failures,
        next_command,
    })
}

fn proof_sprint_next_command(
    manifest_path: &Path,
    manifest: &PilotHarnessManifest,
    qa: &PilotQaOutput,
) -> String {
    if qa.observed_sessions < manifest.target_sessions {
        if let Some((task_id, mode)) = proof_sprint_next_phase_to_collect(manifest) {
            let mode = pilot_session_mode_name(mode);
            return format!(
                "callsieve proof-sprint collect {} --task-id {} --mode {}",
                manifest_path.display(),
                task_id,
                mode
            );
        }
        return format!("callsieve proof-sprint status {}", manifest_path.display());
    }
    if qa.status != "pass" {
        return format!("callsieve pilot-qa {}", manifest_path.display());
    }
    format!(
        "callsieve proof-sprint finalize {} --out benchmarks/evidence/proof.local.json --limit {}",
        manifest_path.display(),
        REHEARSAL_REPORT_LIMIT
    )
}

fn proof_sprint_next_phase_to_collect(
    manifest: &PilotHarnessManifest,
) -> Option<(String, PilotSessionMode)> {
    for task in manifest
        .tasks
        .iter()
        .filter(|task| task.status != "rejected")
    {
        let baseline_exists = Path::new(&task.baseline_trace_path).is_file();
        let callsieve_exists = Path::new(&task.callsieve_trace_path).is_file();
        if baseline_exists && !callsieve_exists {
            return Some((task.id.clone(), PilotSessionMode::Callsieve));
        }
    }
    for task in manifest
        .tasks
        .iter()
        .filter(|task| task.status != "rejected")
    {
        if !Path::new(&task.baseline_trace_path).is_file() {
            return Some((task.id.clone(), PilotSessionMode::Baseline));
        }
    }
    None
}

fn proof_sprint_has_collected_phase(manifest: &PilotHarnessManifest) -> bool {
    manifest.tasks.iter().any(|task| {
        Path::new(&task.baseline_trace_path).is_file()
            || Path::new(&task.callsieve_trace_path).is_file()
            || Path::new(&task.trace_path).is_file()
    })
}

fn proof_sprint_run(
    manifest: &Path,
    max_budget_usd: &str,
    resume: bool,
    limit: usize,
    dry_run: bool,
) -> Result<ProofSprintRunOutput> {
    if limit == 0 {
        anyhow::bail!("proof-sprint run --limit must be greater than 0")
    }
    let manifest_value = read_pilot_manifest(manifest)?;
    if !resume && proof_sprint_has_collected_phase(&manifest_value) {
        anyhow::bail!(
            "proof-sprint manifest already has collected phases; pass --resume to continue"
        )
    }

    let mut phases = Vec::new();
    for _ in 0..limit {
        let current_manifest = read_pilot_manifest(manifest)?;
        let qa = pilot_qa(manifest)?;
        if qa.observed_sessions >= current_manifest.target_sessions || qa.status == "pass" {
            break;
        }
        let Some((task_id, mode)) = proof_sprint_next_phase_to_collect(&current_manifest) else {
            break;
        };
        let collect = proof_sprint_collect(manifest, &task_id, mode, max_budget_usd, dry_run)?;
        let status = collect.status.clone();
        phases.push(ProofSprintRunPhaseOutput {
            task_id,
            mode,
            status,
            collect,
        });
        if dry_run {
            break;
        }
    }

    let status_after = proof_sprint_status(manifest)?;
    let next_command = status_after.next_command.clone();
    let status = if dry_run {
        "dry_run"
    } else if !phases.is_empty() {
        "collected"
    } else {
        status_after.status.as_str()
    };

    Ok(ProofSprintRunOutput {
        command: "proof-sprint run",
        status: status.to_string(),
        manifest: manifest.display().to_string(),
        resume,
        dry_run,
        requested_limit: limit,
        collected_phases: if dry_run { 0 } else { phases.len() },
        phases,
        status_after,
        next_command,
    })
}

fn proof_sprint_collect(
    manifest: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    max_budget_usd: &str,
    dry_run: bool,
) -> Result<ProofSprintCollectOutput> {
    let task = pilot_task_from_manifest(manifest, task_id)?;
    if task.client != "claude" {
        anyhow::bail!("proof-sprint collect currently supports Claude tasks only")
    }
    let model = task.model;
    let collect = collect_claude_observed_session(
        manifest,
        task_id,
        mode,
        &model,
        max_budget_usd,
        None,
        4,
        1,
        Vec::new(),
        dry_run,
    )?;
    Ok(ProofSprintCollectOutput {
        command: "proof-sprint collect",
        status: collect.status.clone(),
        manifest: manifest.display().to_string(),
        task_id: task_id.to_string(),
        mode,
        model,
        collect,
        next_status: format!("callsieve proof-sprint status {}", manifest.display()),
    })
}

fn proof_sprint_finalize(
    manifest: &Path,
    out: &Path,
    limit: usize,
) -> Result<ProofSprintFinalizeOutput> {
    let finalize = pilot_finalize(manifest, out, limit, 2, true)?;
    Ok(ProofSprintFinalizeOutput {
        command: "proof-sprint finalize",
        status: "finalized".to_string(),
        manifest: manifest.display().to_string(),
        out: out.display().to_string(),
        finalize,
    })
}

fn proof_sprint_repos(manifest: &PilotHarnessManifest) -> Vec<String> {
    manifest
        .tasks
        .iter()
        .map(|task| task.repo.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn proof_sprint_client(manifest: &PilotHarnessManifest) -> String {
    let clients = manifest
        .tasks
        .iter()
        .map(|task| task.client.as_str())
        .collect::<BTreeSet<_>>();
    if clients.len() == 1 {
        clients.into_iter().next().unwrap_or("unknown").to_string()
    } else if clients.is_empty() {
        "unknown".to_string()
    } else {
        "mixed".to_string()
    }
}

fn pilot_task_from_manifest(manifest: &Path, task_id: &str) -> Result<PilotHarnessTask> {
    read_pilot_manifest(manifest)?
        .tasks
        .into_iter()
        .find(|task| task.id == task_id)
        .with_context(|| format!("pilot task not found: {task_id}"))
}

fn normalize_claude_allowed_tools(allowed_tools: Vec<String>) -> Vec<String> {
    let mut tools: Vec<String> = allowed_tools
        .into_iter()
        .map(|tool| tool.trim().to_string())
        .filter(|tool| !tool.is_empty())
        .collect();
    if tools.is_empty() {
        tools = vec!["Glob".to_string(), "Grep".to_string(), "Read".to_string()];
    }
    tools
}

fn claude_observed_prompt_plan(
    repo: &Path,
    task: &PilotHarnessTask,
    mode: PilotSessionMode,
    context_limit: usize,
    snippets_per_file: usize,
) -> Result<PilotPromptPlan> {
    match mode {
        PilotSessionMode::Baseline => Ok(PilotPromptPlan {
            command: format!("baseline prompt without CallSieve for task {:?}", task.task),
            files_read: Vec::new(),
            context_selected_files: Vec::new(),
            prompt: format!(
                "Observed baseline measurement. Do not use CallSieve, callsieve MCP tools, or callsieve commands. Do not edit files. Use only normal code search and file reading. Task: {}\n\nUse Glob, Grep, and Read to inspect the repo. You must Read every file you rely on before answering. Return JSON only with files_read containing the actual Read tool paths, would_change, and a concise rationale.",
                task.task
            ),
        }),
        PilotSessionMode::Callsieve => {
            let index = load_or_build_index(repo)?;
            let mut plan =
                build_callsieve_prompt_plan(repo, &index, task, context_limit, snippets_per_file)?;
            plan.prompt = format!(
                "Observed CallSieve measurement. Do not edit files. Use the compact CallSieve context as primary evidence. Treat read_first as files selected into context, not as a mandatory Read list. Call Glob, Grep, or Read only if the packet is insufficient.\n\n{}",
                plan.prompt
            );
            Ok(plan)
        }
    }
}

fn claude_observed_command_summary(
    repo: &Path,
    task: &str,
    mode: PilotSessionMode,
    model: &str,
    max_budget_usd: &str,
    allowed_tools: &[String],
) -> String {
    let prompt_summary = match mode {
        PilotSessionMode::Baseline => {
            format!("baseline prompt without CallSieve for task {task:?}")
        }
        PilotSessionMode::Callsieve => {
            format!(
                "callsieve agent-context {} {:?} followed by Claude prompt",
                repo.display(),
                task
            )
        }
    };
    format!(
        "claude -p --input-format text <{}> --model {} --output-format stream-json --verbose --no-session-persistence --max-budget-usd {} --tools {}",
        prompt_summary,
        model,
        max_budget_usd,
        allowed_tools.join(",")
    )
}

#[allow(clippy::too_many_arguments)]
fn record_observed_session(
    command_name: &'static str,
    manifest: &Path,
    client: Option<AgentClient>,
    model: Option<&str>,
    task_id: &str,
    mode: PilotSessionMode,
    event_command: &str,
    tokens: Option<usize>,
    usage_json: Option<&Path>,
    files_read: Vec<String>,
    context_selected_files: Vec<String>,
    dry_run: bool,
) -> Result<RecordObservedSessionOutput> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        anyhow::bail!("task_id is required")
    }
    let event_command = event_command.trim();
    if event_command.is_empty() {
        anyhow::bail!("command is required")
    }
    let token_input = observed_token_input(tokens, usage_json)?;
    let mut files_read: Vec<String> = files_read
        .into_iter()
        .map(|file| file.trim().to_string())
        .filter(|file| !file.is_empty())
        .collect();
    if files_read.is_empty()
        && let Some(path) = usage_json
    {
        files_read = claude_code_stream_read_files(path)?;
    }
    files_read = normalize_observed_files_read(manifest, task_id, files_read);
    let context_selected_files = normalize_observed_files_read(
        manifest,
        task_id,
        context_selected_files
            .into_iter()
            .map(|file| file.trim().to_string())
            .filter(|file| !file.is_empty())
            .collect(),
    );
    let context_covers_callsieve =
        mode == PilotSessionMode::Callsieve && !context_selected_files.is_empty();
    if files_read.is_empty() && !context_covers_callsieve {
        anyhow::bail!("files_read must include at least one file actually read")
    }

    let token_evidence = observed_token_evidence(&token_input);
    let pilot_run_command = record_observed_pilot_run_command(
        manifest,
        task_id,
        mode,
        event_command,
        token_input.tokens,
        &files_read,
        &context_selected_files,
    );
    let pilot_run = if dry_run {
        None
    } else {
        Some(serde_json::to_value(
            pilot_run_with_context_and_token_evidence(
                manifest,
                task_id,
                mode,
                event_command,
                files_read.clone(),
                context_selected_files.clone(),
                token_input.tokens,
                Some(&token_evidence),
            )?,
        )?)
    };
    let status = if dry_run { "dry_run" } else { "recorded" }.to_string();

    Ok(RecordObservedSessionOutput {
        command: command_name,
        status,
        manifest: manifest.display().to_string(),
        client: client
            .map(agent_client_name)
            .unwrap_or("unregistered")
            .to_string(),
        model: model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string),
        task_id: task_id.to_string(),
        mode,
        files_read,
        context_selected_files,
        tokens: token_input.tokens,
        token_accounting_source: "transcript_context_tokens",
        token_input_source: token_input.token_input_source,
        usage_json: token_input.usage_json,
        usage_breakdown: token_input.usage_breakdown,
        pilot_run_command,
        pilot_run,
        next_qa: format!("callsieve pilot-qa {}", manifest.display()),
    })
}

fn observed_token_input(
    tokens: Option<usize>,
    usage_json: Option<&Path>,
) -> Result<ObservedTokenInput> {
    match (tokens, usage_json) {
        (Some(_), Some(_)) => {
            anyhow::bail!("pass either --tokens or --usage-json, not both")
        }
        (Some(tokens), None) => {
            if tokens == 0 {
                anyhow::bail!(
                    "tokens must be a positive transcript context token count. Do not estimate tokens."
                )
            }
            Ok(ObservedTokenInput {
                tokens,
                token_input_source: "manual_transcript_context_tokens".to_string(),
                usage_json: None,
                usage_breakdown: None,
            })
        }
        (None, Some(path)) => claude_code_usage_token_input(path),
        (None, None) => {
            anyhow::bail!("tokens are required. Pass --tokens or --usage-json.")
        }
    }
}

fn claude_code_usage_token_input(path: &Path) -> Result<ObservedTokenInput> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("failed to read Claude Code usage JSON: {}", path.display()))?;
    let values = parse_claude_code_artifact(path, &json)?;
    let usage = values
        .iter()
        .rev()
        .find_map(|value| value.get("usage"))
        .context("Claude Code usage artifact is missing a usage object")?;
    let input_tokens = json_field_usize(usage, "input_tokens");
    let cache_creation_input_tokens = json_field_usize(usage, "cache_creation_input_tokens");
    let cache_read_input_tokens = json_field_usize(usage, "cache_read_input_tokens");
    let output_tokens = json_field_usize(usage, "output_tokens");
    let total_tokens = input_tokens
        .checked_add(cache_creation_input_tokens)
        .and_then(|total| total.checked_add(cache_read_input_tokens))
        .and_then(|total| total.checked_add(output_tokens))
        .context("Claude Code usage token total overflowed usize")?;
    if total_tokens == 0 {
        anyhow::bail!("Claude Code usage JSON reported zero total tokens")
    }

    Ok(ObservedTokenInput {
        tokens: total_tokens,
        token_input_source: "claude_code_usage_total_tokens".to_string(),
        usage_json: Some(path.display().to_string()),
        usage_breakdown: Some(ClaudeCodeUsageBreakdown {
            input_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
            output_tokens,
            total_tokens,
        }),
    })
}

fn parse_claude_code_artifact(path: &Path, json: &str) -> Result<Vec<serde_json::Value>> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(json) {
        return Ok(vec![value]);
    }

    let mut values = Vec::new();
    for (index, line) in json.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).with_context(|| {
            format!(
                "failed to parse Claude Code stream JSON line {} in {}",
                index + 1,
                path.display()
            )
        })?;
        values.push(value);
    }
    if values.is_empty() {
        anyhow::bail!("Claude Code usage artifact is empty: {}", path.display())
    }
    Ok(values)
}

fn json_field_usize(value: &serde_json::Value, field: &str) -> usize {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}

fn claude_code_stream_read_files(path: &Path) -> Result<Vec<String>> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("failed to read Claude Code stream JSON: {}", path.display()))?;
    let values = parse_claude_code_artifact(path, &json)?;
    let mut files = BTreeSet::new();
    for value in &values {
        collect_claude_read_tool_paths(value, &mut files);
    }
    Ok(files.into_iter().collect())
}

fn collect_claude_read_tool_paths(value: &serde_json::Value, files: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(object) => {
            if object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "tool_use")
                && object
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|name| name == "Read")
                && let Some(path) = object
                    .get("input")
                    .and_then(|input| input.get("file_path").or_else(|| input.get("path")))
                    .and_then(serde_json::Value::as_str)
            {
                let path = path.trim();
                if !path.is_empty() {
                    files.insert(path.to_string());
                }
            }
            for child in object.values() {
                collect_claude_read_tool_paths(child, files);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_claude_read_tool_paths(item, files);
            }
        }
        _ => {}
    }
}

fn normalize_observed_files_read(
    manifest: &Path,
    task_id: &str,
    files_read: Vec<String>,
) -> Vec<String> {
    let repo_root = read_pilot_manifest(manifest)
        .ok()
        .and_then(|manifest| {
            manifest
                .tasks
                .into_iter()
                .find(|task| task.id == task_id)
                .map(|task| PathBuf::from(task.repo))
        })
        .and_then(|path| path.canonicalize().ok());
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for file in files_read {
        let file = observed_file_path_for_manifest(&file, repo_root.as_deref());
        if !file.is_empty() && seen.insert(file.clone()) {
            normalized.push(file);
        }
    }
    normalized
}

fn observed_file_path_for_manifest(path: &str, repo_root: Option<&Path>) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let path = Path::new(trimmed);
    if path.is_absolute()
        && let Some(repo_root) = repo_root
    {
        let path_for_prefix = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if let Ok(relative) = path_for_prefix.strip_prefix(repo_root) {
            return relative.to_string_lossy().replace('\\', "/");
        }
    }
    trimmed.replace('\\', "/")
}

fn observed_token_evidence(token_input: &ObservedTokenInput) -> serde_json::Value {
    let mut evidence = serde_json::json!({
        "accounting_source": "transcript_context_tokens",
        "input_source": token_input.token_input_source,
        "tokens": token_input.tokens
    });
    let object = evidence
        .as_object_mut()
        .expect("token evidence must be a JSON object");
    if let Some(path) = &token_input.usage_json {
        object.insert(
            "usage_json".to_string(),
            serde_json::Value::String(path.clone()),
        );
    }
    if let Some(breakdown) = &token_input.usage_breakdown {
        object.insert(
            "claude_code_usage".to_string(),
            serde_json::to_value(breakdown).expect("Claude Code usage breakdown must serialize"),
        );
    }
    evidence
}

fn record_observed_pilot_run_command(
    manifest: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    event_command: &str,
    tokens: usize,
    files_read: &[String],
    context_selected_files: &[String],
) -> String {
    let mut args = vec![
        "pilot-run".to_string(),
        manifest.display().to_string(),
        "--task-id".to_string(),
        task_id.to_string(),
        "--mode".to_string(),
        pilot_session_mode_name(mode).to_string(),
        "--command".to_string(),
        event_command.to_string(),
    ];
    for file in files_read {
        args.push("--files-read".to_string());
        args.push(file.clone());
    }
    for file in context_selected_files {
        args.push("--context-selected-file".to_string());
        args.push(file.clone());
    }
    args.push("--tokens".to_string());
    args.push(tokens.to_string());
    format_callsieve_command(&args)
}

fn pilot_session_mode_name(mode: PilotSessionMode) -> &'static str {
    match mode {
        PilotSessionMode::Baseline => "baseline",
        PilotSessionMode::Callsieve => "callsieve",
    }
}

fn format_callsieve_command(args: &[String]) -> String {
    std::iter::once("callsieve".to_string())
        .chain(args.iter().map(|arg| quote_cli_arg(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_shell_command(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_cli_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_cli_arg(arg: &str) -> String {
    if !arg.is_empty()
        && arg.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | ':' | '=')
        })
    {
        return arg.to_string();
    }
    format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
}

fn perf_report(
    root: &Path,
    tasks_path: Option<&Path>,
    iterations: usize,
) -> Result<PerfReportOutput> {
    let iterations = iterations.max(1);
    let tasks = perf_tasks(tasks_path)?;
    let mut all_samples = Vec::new();
    let mut task_outputs = Vec::new();

    for task in tasks {
        let mut samples = Vec::new();
        for _ in 0..iterations {
            let started = Instant::now();
            let (index, _) = load_index_timed(root)?;
            let _context = query::build_context(root, &index, &task, 8, 2, true)?;
            samples.push(duration_ms(started.elapsed()));
        }
        all_samples.extend(samples.iter().copied());
        let summary = latency_summary(&samples);
        task_outputs.push(PerfTaskOutput {
            task,
            samples: summary.samples,
            p50_ms: summary.p50_ms,
            p95_ms: summary.p95_ms,
            min_ms: summary.min_ms,
            max_ms: summary.max_ms,
        });
    }

    let summary = latency_summary(&all_samples);
    Ok(PerfReportOutput {
        command: "perf-report",
        status: if summary.p95_ms <= 1_000 {
            "pass"
        } else {
            "warn"
        }
        .to_string(),
        root: root_label(root),
        iterations,
        task_count: task_outputs.len(),
        summary,
        tasks: task_outputs,
    })
}

fn perf_tasks(tasks_path: Option<&Path>) -> Result<Vec<String>> {
    let Some(tasks_path) = tasks_path else {
        return Ok(default_perf_tasks());
    };
    let tasks_json = fs::read_to_string(tasks_path).with_context(|| {
        format!(
            "failed to read perf task manifest: {}",
            tasks_path.display()
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&tasks_json).with_context(|| {
        format!(
            "failed to parse perf task manifest: {}",
            tasks_path.display()
        )
    })?;
    let tasks_value = value.get("tasks").unwrap_or(&value);
    let tasks = tasks_value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str().map(str::to_string).or_else(|| {
                item.get("task")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
        })
        .collect::<Vec<_>>();

    if tasks.is_empty() {
        Ok(default_perf_tasks())
    } else {
        Ok(tasks)
    }
}

fn default_perf_tasks() -> Vec<String> {
    [
        "change agent context retrieval timing and debug output",
        "fix exact symbol ranking for context generation",
        "update proof report evidence gates",
        "add CLI integration tests for retrieval evaluation",
        "update MCP setup docs for context-first agents",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn latency_summary(samples: &[u64]) -> PerfLatencySummary {
    if samples.is_empty() {
        return PerfLatencySummary {
            samples: 0,
            p50_ms: 0,
            p95_ms: 0,
            min_ms: 0,
            max_ms: 0,
        };
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    PerfLatencySummary {
        samples: sorted.len(),
        p50_ms: percentile_ms(&sorted, 0.50),
        p95_ms: percentile_ms(&sorted, 0.95),
        min_ms: *sorted.first().unwrap_or(&0),
        max_ms: *sorted.last().unwrap_or(&0),
    }
}

fn percentile_ms(sorted_samples: &[u64], percentile: f64) -> u64 {
    let len = sorted_samples.len();
    if len == 0 {
        return 0;
    }
    let index = ((len as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(len - 1);
    sorted_samples[index]
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn init_tracing(verbose: bool) -> Result<()> {
    if verbose {
        tracing_subscriber::fmt()
            .with_env_filter("callsieve=debug")
            .try_init()
            .ok();
    }
    Ok(())
}

fn root_label(path: &Path) -> String {
    if path == Path::new(".") {
        ".".to_string()
    } else {
        path.display().to_string()
    }
}

fn repo_relative_display(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn refresh_watch_index(
    path: &Path,
    watch_status: &str,
    mode: &str,
    lsp: bool,
) -> Result<WatchOutput> {
    let next_generation = store::json_store::load_index(path)
        .ok()
        .map(|index| index.metadata.index_generation.saturating_add(1))
        .unwrap_or(1);
    let index = indexer::build_index_with_options(
        path,
        indexer::IndexOptions {
            lsp,
            watch_status: watch_status.to_string(),
            watcher_mode: mode.to_string(),
            index_generation: next_generation,
            last_error: None,
        },
    )?;
    store::json_store::save_index(path, &index)?;
    let status = query::index_status(path, Some(&index));

    Ok(WatchOutput {
        command: "watch",
        root: root_label(path),
        mode: mode.to_string(),
        refreshed: true,
        status,
    })
}

fn build_index_output(root: &Path, lsp: bool) -> Result<IndexOutput> {
    let index = if lsp {
        indexer::build_index_with_options(
            root,
            indexer::IndexOptions {
                lsp,
                ..indexer::IndexOptions::default()
            },
        )?
    } else {
        indexer::build_index(root)?
    };
    let index_path = store::json_store::save_index(root, &index)?;
    Ok(IndexOutput {
        command: "index",
        root: root_label(root),
        index: repo_relative_display(root, &index_path),
        files: index.files.len(),
        symbols: index.symbols.len(),
        imports: index.imports.len(),
        references: index.references.len(),
        lsp_enriched: index.metadata.lsp_enriched,
        warnings: index.warnings,
    })
}

fn bootstrap(
    root: &Path,
    client: AgentClient,
    strict: bool,
    force: bool,
    lsp: bool,
) -> Result<BootstrapOutput> {
    let mut steps = Vec::new();
    let mut generated_files = Vec::new();

    let index = build_index_output(root, lsp)?;
    generated_files.push(index.index.clone());
    steps.push(automation_step(
        "index",
        "pass",
        format!(
            "indexed {} files, {} symbols, {} references",
            index.files, index.symbols, index.references
        ),
    ));

    let setup = setup_agent(client, root, force)?;
    generated_files.extend(setup.files.clone());
    steps.push(automation_step(
        "agent_setup",
        "pass",
        format!("wrote {} agent setup file(s)", setup.files.len()),
    ));

    if matches!(client, AgentClient::Codex) {
        let launchers = write_codex_launchers(root, &setup.first_required_command, force)?;
        generated_files.extend(launchers.clone());
        steps.push(automation_step(
            "codex_launchers",
            "pass",
            format!("wrote {} Codex launcher file(s)", launchers.len()),
        ));
        let (_, _, hook_files) = write_codex_hooks_files(root, strict, force, 6, 1)?;
        generated_files.extend(hook_files.clone());
        steps.push(automation_step(
            "codex_hooks",
            "pass",
            format!("wrote {} Codex hook file(s)", hook_files.len()),
        ));
    }
    if matches!(client, AgentClient::Claude) {
        let (_, _, hook_files) = write_claude_hooks_files(root, strict, force, 6, 1)?;
        generated_files.extend(hook_files.clone());
        steps.push(automation_step(
            "claude_hooks",
            "pass",
            format!("wrote {} Claude Code hook file(s)", hook_files.len()),
        ));
    }
    if let Some(hook_client) = hook_client_for_agent(client) {
        let (_, _, hook_files) = write_client_hooks_files(root, hook_client, strict, force, 6, 1)?;
        generated_files.extend(hook_files.clone());
        steps.push(automation_step(
            format!("{}_hooks", hook_client_name(hook_client)),
            "pass",
            format!(
                "wrote {} {} hook file(s)",
                hook_files.len(),
                hook_client_display(hook_client)
            ),
        ));
    }

    let daemon = run_daemon(root, lsp, 1000, false, false)?;
    steps.push(automation_step(
        "daemon",
        "pass",
        format!("daemon state is {}", daemon.state.status),
    ));

    if strict {
        let shim = install_shim(root, force, true)?;
        generated_files.extend(shim.files.clone());
        steps.push(automation_step(
            "strict_shim",
            "pass",
            format!("installed {} strict shim file(s)", shim.files.len()),
        ));
    }

    generated_files.sort();
    generated_files.dedup();
    let enforcement = enforce_setup(root, client, None, strict, strict)?;
    let status = enforcement.status.clone();

    Ok(BootstrapOutput {
        command: "bootstrap",
        status,
        root: root_label(root),
        client: agent_client_name(client).to_string(),
        strict,
        steps,
        generated_files,
        daemon: daemon.state,
        first_required_command: setup.first_required_command,
        enforcement,
    })
}

fn doctor(root: &Path, client: AgentClient, fix: bool, strict: bool) -> Result<DoctorOutput> {
    let mut fixes = Vec::new();
    let mut checks = doctor_checks(root, client, strict)?;
    if fix {
        if check_failed(&checks, "fresh_index") {
            let index = build_index_output(root, false)?;
            fixes.push(automation_step(
                "index",
                "pass",
                format!("rebuilt index at {}", index.index),
            ));
        }
        if checks
            .iter()
            .any(|check| check.check.starts_with("agent_file:") && check.status == "fail")
        {
            let setup = setup_missing_agent_files(client, root)?;
            fixes.push(automation_step(
                "agent_setup",
                "pass",
                format!("wrote {} missing agent setup file(s)", setup.files.len()),
            ));
        }
        if matches!(client, AgentClient::Codex)
            && checks
                .iter()
                .any(|check| check.check.starts_with("codex_bootstrap:") && check.status == "fail")
        {
            let first_required_command =
                format!("callsieve agent-context {} \"<task>\"", root.display());
            let launchers = write_codex_launchers(root, &first_required_command, false)?;
            fixes.push(automation_step(
                "codex_launchers",
                "pass",
                format!("wrote {} missing Codex launcher file(s)", launchers.len()),
            ));
        }
        if matches!(client, AgentClient::Codex)
            && checks
                .iter()
                .any(|check| check.check.starts_with("codex_hooks") && check.status == "fail")
        {
            let (_, _, hook_files) = write_codex_hooks_files(root, strict, true, 6, 1)?;
            fixes.push(automation_step(
                "codex_hooks",
                "pass",
                format!("wrote {} missing Codex hook file(s)", hook_files.len()),
            ));
        }
        if matches!(client, AgentClient::Claude)
            && checks
                .iter()
                .any(|check| check.check.starts_with("claude_hooks") && check.status == "fail")
        {
            let (_, _, hook_files) = write_claude_hooks_files(root, strict, true, 6, 1)?;
            fixes.push(automation_step(
                "claude_hooks",
                "pass",
                format!(
                    "wrote {} missing Claude Code hook file(s)",
                    hook_files.len()
                ),
            ));
        }
        if let Some(hook_client) = hook_client_for_agent(client)
            && checks.iter().any(|check| {
                check
                    .check
                    .starts_with(&format!("{}_hooks", hook_client_name(hook_client)))
                    && check.status == "fail"
            })
        {
            let (_, _, hook_files) =
                write_client_hooks_files(root, hook_client, strict, true, 6, 1)?;
            fixes.push(automation_step(
                format!("{}_hooks", hook_client_name(hook_client)),
                "pass",
                format!(
                    "wrote {} missing {} hook file(s)",
                    hook_files.len(),
                    hook_client_display(hook_client)
                ),
            ));
        }
        if check_failed(&checks, "daemon_state") {
            let daemon = run_daemon(root, false, 1000, false, false)?;
            fixes.push(automation_step(
                "daemon",
                "pass",
                format!("daemon state is {}", daemon.state.status),
            ));
        }
        if strict && check_failed(&checks, "shim_files") {
            let shim = install_shim(root, true, true)?;
            fixes.push(automation_step(
                "strict_shim",
                "pass",
                format!("installed {} strict shim file(s)", shim.files.len()),
            ));
        }
        checks = doctor_checks(root, client, strict)?;
    }

    let status = status_from_checks(&checks);
    let message = if status == "pass" {
        "all local CallSieve adoption checks passed".to_string()
    } else if fix {
        "some CallSieve adoption checks still failed after repair".to_string()
    } else {
        "run doctor again with --fix to repair failed local checks".to_string()
    };

    Ok(DoctorOutput {
        command: "doctor",
        status,
        root: root_label(root),
        client: agent_client_name(client).to_string(),
        message,
        checks,
        fixes,
    })
}

fn begin_task(
    root: &Path,
    task: &str,
    client: AgentClient,
    trace_out: Option<&Path>,
    proof_trace: bool,
    limit: usize,
    snippets_per_file: usize,
) -> Result<BeginOutput> {
    let (index, index_load_ms, refreshed) = load_fresh_index_timed(root)?;
    let mut context = query::build_context(root, &index, task, limit, snippets_per_file, true)?;
    context.add_index_load_time(index_load_ms);
    if refreshed {
        context.add_warning("rebuilt missing or stale CallSieve index before context");
    }
    let context_value = serde_json::to_value(&context)?;
    let context_selected_files = context_read_first_files(&context_value);
    let tokens = serde_json::to_string(&context_value)
        .map(|json| json.len().div_ceil(4))
        .unwrap_or_default();
    let command = first_required_context_command(root, task);

    let (trace_path, trace_event) = if let Some(trace) = trace_out {
        session_start(
            root,
            task,
            client,
            default_agent_model(client),
            trace,
            context_selected_files.clone(),
            context_selected_files.clone(),
        )?;
        if proof_trace {
            mark_explicit_proof_trace(trace)?;
        }
        let event = session_event_with_token_evidence(
            trace,
            &command,
            Vec::new(),
            context_selected_files,
            Some(tokens),
            Some(SessionPhase::Callsieve),
            None,
        )?
        .event;
        (Some(trace.display().to_string()), event)
    } else {
        (
            None,
            serde_json::json!({
                "timestamp": now_unix_seconds(),
                "command": command,
                "files_read": [],
                "context_selected_files": context_selected_files,
                "tokens": tokens,
                "classification": "callsieve_context",
                "phase": "callsieve"
            }),
        )
    };

    let next_step = if let Some(trace_path) = trace_path.as_deref() {
        if proof_trace {
            format!(
                "Read read_first files, review context.sel.next, use instruction.x.o/n/next and local expansion commands for focused detail, then broad grep only if insufficient; append explicit session-event records with tokens and phase, then audit with `callsieve trace-check {trace_path} --strict`."
            )
        } else {
            format!(
                "Read read_first files, review context.sel.next, use instruction.x.o/n/next and local expansion commands for focused detail, then broad grep only if insufficient; audit with `callsieve trace-check {trace_path} --strict`."
            )
        }
    } else {
        "Read read_first files, review context.sel.next, use instruction.x.o/n/next and local expansion commands for focused detail, then broad grep only if insufficient; pass --trace-out to record an audited trace."
            .to_string()
    };
    let proof_next_commands = if proof_trace {
        trace_path
            .as_deref()
            .map(|trace_path| {
                vec![
                    format!(
                        "callsieve session-event {trace_path} --phase callsieve --tokens <transcript_context_tokens> --command \"<command>\" --context-selected-file <file>"
                    ),
                    format!("callsieve trace-check {trace_path} --strict"),
                ]
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(BeginOutput {
        command: "begin",
        root: root_label(root),
        client: agent_client_name(client).to_string(),
        task: task.to_string(),
        policy: "context_first; read returned files before broad grep or repeated file reads",
        local_first_expansion: agent_local_first_expansion(root, &context),
        next_step,
        proof_next_commands,
        context,
        trace_event,
        trace_path,
    })
}

fn doctor_checks(root: &Path, client: AgentClient, strict: bool) -> Result<Vec<EnforceCheck>> {
    let mut checks = Vec::new();
    let index = store::json_store::load_index(root).ok();
    let status = query::index_status(root, index.as_ref());
    let status_value = serde_json::to_value(&status)?;
    let fresh = status_value
        .get("fresh")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    checks.push(enforce_check(
        "fresh_index",
        fresh,
        if fresh {
            "index is fresh"
        } else {
            "index is missing or stale"
        },
    ));

    for (path, _) in agent_files(client, root) {
        let relative = repo_relative_display(root, &path);
        checks.push(enforce_check(
            format!("agent_file:{relative}"),
            path.is_file(),
            if path.is_file() {
                "required agent policy/config exists".to_string()
            } else {
                "required agent policy/config is missing".to_string()
            },
        ));
    }

    if matches!(client, AgentClient::Codex) && strict {
        for path in codex_launcher_paths(root) {
            let relative = repo_relative_display(root, &path);
            checks.push(enforce_check(
                format!("codex_bootstrap:{relative}"),
                path.is_file(),
                if path.is_file() {
                    "Codex bootstrap launcher exists".to_string()
                } else {
                    "Codex bootstrap launcher is missing".to_string()
                },
            ));
        }
        let codex_hooks = codex_hooks_doctor(root, true);
        checks.push(enforce_check(
            "codex_hooks",
            codex_hooks.status == "pass",
            if codex_hooks.status == "pass" {
                "Codex lifecycle hooks are installed"
            } else {
                "Codex lifecycle hooks are missing or stale"
            },
        ));
    }
    if matches!(client, AgentClient::Claude) && strict {
        let claude_hooks = claude_hooks_doctor(root, true);
        checks.push(enforce_check(
            "claude_hooks",
            claude_hooks.status == "pass",
            if claude_hooks.status == "pass" {
                "Claude Code lifecycle hooks are installed"
            } else {
                "Claude Code lifecycle hooks are missing or stale"
            },
        ));
    }
    if let Some(hook_client) = hook_client_for_agent(client)
        && strict
    {
        let hooks = client_hooks_doctor(root, hook_client, true);
        checks.push(enforce_check(
            format!("{}_hooks", hook_client_name(hook_client)),
            hooks.status == "pass",
            if hooks.status == "pass" {
                format!("{} hooks are installed", hook_client_display(hook_client))
            } else {
                format!(
                    "{} hooks are missing or stale",
                    hook_client_display(hook_client)
                )
            },
        ));
    }

    let daemon = load_daemon_state(root).unwrap_or_else(|| missing_daemon_state(root));
    let daemon_ok = daemon_state_is_usable(&daemon);
    checks.push(enforce_check(
        "daemon_state",
        daemon_ok,
        if daemon_ok {
            format!("daemon state is {}", daemon.status)
        } else {
            "daemon state is missing, stopped, or errored".to_string()
        },
    ));

    let shim_files = shim_files_installed(root);
    checks.push(check_with_status(
        "shim_files",
        if shim_files {
            "pass"
        } else if strict {
            "fail"
        } else {
            "warn"
        },
        if shim_files {
            "grep shim files are installed"
        } else if strict {
            "strict mode requires project-local rg/grep shims"
        } else {
            "grep shim files are optional unless --strict is used"
        },
    ));

    let bin_dir = shim_bin_dir(root);
    let shim_on_path = shim_dir_on_path(&bin_dir);
    checks.push(check_with_status(
        "path_contains_shim_dir",
        if shim_on_path { "pass" } else { "warn" },
        if shim_on_path {
            "shim bin directory is on PATH"
        } else {
            "prepend shim bin directory to PATH before running agents"
        },
    ));

    Ok(checks)
}

fn setup_missing_agent_files(client: AgentClient, root: &Path) -> Result<SetupAgentOutput> {
    let first_required_command = format!("callsieve agent-context {} \"<task>\"", root.display());
    let mut written = Vec::new();
    for (path, content) in agent_files(client, root) {
        if !path.exists() {
            write_project_file(root, &path, &content, false, &mut written)?;
        }
    }

    Ok(SetupAgentOutput {
        command: "setup-agent",
        client: agent_client_name(client).to_string(),
        root: root_label(root),
        files: written,
        first_required_command,
        policy: "Call callsieve_context before broad grep, rg, repository search, or repeated file reads. Read read_first, review context.sel.next, and use instruction.x.o/n/next plus focus, related, and tests before grep.",
        warnings: agent_client_warnings_for_root(client, root),
    })
}

fn default_agent_model(client: AgentClient) -> &'static str {
    match client {
        AgentClient::Codex => "gpt-5-codex",
        AgentClient::Claude => "claude",
        AgentClient::Copilot => "copilot",
        AgentClient::OpenCode => "opencode",
        AgentClient::Antigravity => "antigravity",
        AgentClient::Cursor => "cursor",
        AgentClient::Vscode => "vscode",
        AgentClient::Windsurf => "windsurf",
        AgentClient::Continue => "continue",
        AgentClient::Zed => "zed",
        AgentClient::Junie => "junie",
        AgentClient::JetBrains => "jetbrains",
        AgentClient::Amp => "amp",
        AgentClient::Goose => "goose",
        AgentClient::Warp => "warp",
        AgentClient::Cline => "cline",
        AgentClient::Zoo => "zoo",
        AgentClient::Roo => "roo",
        AgentClient::Generic => "generic-agent",
    }
}

fn automation_step(
    step: impl Into<String>,
    status: impl Into<String>,
    message: impl Into<String>,
) -> AutomationStep {
    AutomationStep {
        step: step.into(),
        status: status.into(),
        message: message.into(),
    }
}

fn check_failed(checks: &[EnforceCheck], name: &str) -> bool {
    checks
        .iter()
        .any(|check| check.check == name && check.status == "fail")
}

fn status_from_checks(checks: &[EnforceCheck]) -> String {
    if checks.iter().all(|check| check.status != "fail") {
        "pass"
    } else {
        "fail"
    }
    .to_string()
}

fn check_with_status(
    check: impl Into<String>,
    status: impl Into<String>,
    message: impl Into<String>,
) -> EnforceCheck {
    EnforceCheck {
        check: check.into(),
        status: status.into(),
        message: message.into(),
    }
}

fn session_start(
    root: &Path,
    task: &str,
    client: AgentClient,
    model: &str,
    trace: &Path,
    expected_files: Vec<String>,
    critical_files: Vec<String>,
) -> Result<SessionStartOutput> {
    if let Some(parent) = trace
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let first_required_command = first_required_context_command(root, task);
    let mut value = serde_json::json!({
        "metadata": {
            "collection": "observed_session",
            "client": agent_client_name(client),
            "model": model,
            "repo": root_label(root),
            "started_at": now_unix_seconds(),
            "updated_at": now_unix_seconds()
        },
        "task": task,
        "expected_files": expected_files,
        "critical_files": critical_files,
        "baseline": empty_session_metrics(),
        "callsieve": empty_session_metrics(),
        "session": {
            "baseline": empty_session_metrics(),
            "callsieve": empty_session_metrics()
        },
        "events": [],
        "misses": [],
        "token_accounting": {
            "source": "transcript_context_tokens",
            "fallback_policy": "local_estimator_allowed_only_when_labeled_separately",
            "baseline_tokens": 0,
            "callsieve_tokens": 0,
            "token_savings": 0,
            "token_reduction_percent": 0.0
        },
        "policy": {
            "context_first": true,
            "strict_trace_check": true,
            "first_required_command": first_required_command
        }
    });
    normalize_session_trace(&mut value)?;
    fs::write(trace, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", trace.display()))?;

    Ok(SessionStartOutput {
        command: "session-start",
        trace: trace.display().to_string(),
        task: task.to_string(),
        client: agent_client_name(client).to_string(),
        model: model.to_string(),
        collection: "observed_session",
        first_required_command,
    })
}

fn mark_explicit_proof_trace(trace: &Path) -> Result<()> {
    let mut value = read_trace_value(trace)?;
    let object = value
        .as_object_mut()
        .context("session trace root must be a JSON object")?;
    if let Some(metadata) = object
        .get_mut("metadata")
        .and_then(serde_json::Value::as_object_mut)
    {
        metadata.insert(
            "proof_trace_source".to_string(),
            serde_json::json!("explicit_callsieve_begin"),
        );
        metadata.insert(
            "updated_at".to_string(),
            serde_json::json!(now_unix_seconds()),
        );
    }
    if let Some(policy) = object
        .get_mut("policy")
        .and_then(serde_json::Value::as_object_mut)
    {
        policy.insert("proof_mode".to_string(), serde_json::json!(true));
        policy.insert(
            "post_tool_hook_required".to_string(),
            serde_json::json!(false),
        );
        policy.insert(
            "event_source".to_string(),
            serde_json::json!("explicit_session_events"),
        );
    }
    fs::write(trace, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", trace.display()))
}

fn trace_proof_mode(value: &serde_json::Value) -> bool {
    value
        .get("policy")
        .and_then(|policy| policy.get("proof_mode"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn session_event_with_token_evidence(
    trace: &Path,
    command: &str,
    files_read: Vec<String>,
    context_selected_files: Vec<String>,
    tokens: Option<usize>,
    phase: Option<SessionPhase>,
    token_evidence: Option<&serde_json::Value>,
) -> Result<SessionEventOutput> {
    let mut value = read_trace_value(trace)?;
    let proof_mode = trace_proof_mode(&value);
    if proof_mode && tokens.is_none() {
        anyhow::bail!(
            "proof-mode traces require --tokens on every session-event after begin --proof-trace"
        );
    }
    if proof_mode && phase.is_none() {
        anyhow::bail!(
            "proof-mode traces require explicit --phase baseline|callsieve on every session-event after begin --proof-trace"
        );
    }
    let classification = classify_session_command(command);
    if proof_mode && classification == "file_read" && files_read.is_empty() {
        anyhow::bail!("proof-mode file-read events require at least one --files-read value");
    }
    let phase_name = phase
        .map(session_phase_name)
        .map(str::to_string)
        .unwrap_or_else(|| infer_session_phase(&value, command).to_string());
    let mut event = serde_json::json!({
        "timestamp": now_unix_seconds(),
        "command": command,
        "files_read": files_read,
        "tokens": tokens,
        "classification": classification,
        "phase": phase_name
    });
    if !context_selected_files.is_empty() {
        event
            .as_object_mut()
            .context("session event must be a JSON object")?
            .insert(
                "context_selected_files".to_string(),
                serde_json::json!(context_selected_files),
            );
    }
    if let Some(token_evidence) = token_evidence {
        event
            .as_object_mut()
            .context("session event must be a JSON object")?
            .insert("token_evidence".to_string(), token_evidence.clone());
    }
    let object = value
        .as_object_mut()
        .context("session trace root must be a JSON object")?;
    let events = object
        .entry("events")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("session trace events must be an array")?;
    events.push(event.clone());
    normalize_session_trace(&mut value)?;
    fs::write(trace, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", trace.display()))?;
    let summary = query::trace_summary_from_str(&serde_json::to_string(&value)?)?;

    Ok(SessionEventOutput {
        command: "session-event",
        trace: trace.display().to_string(),
        event,
        summary,
    })
}

fn session_finish(
    trace: &Path,
    out: &Path,
    ground_truth_files: &[String],
) -> Result<SessionFinishOutput> {
    let mut value = read_trace_value(trace)?;
    normalize_session_trace(&mut value)?;
    if let Some(metadata) = value
        .get_mut("metadata")
        .and_then(serde_json::Value::as_object_mut)
    {
        metadata.insert(
            "finished_at".to_string(),
            serde_json::Value::from(now_unix_seconds()),
        );
    }
    fs::write(trace, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", trace.display()))?;
    let summary = query::trace_summary_from_str(&serde_json::to_string(&value)?)?;
    let ground_truth_metrics = if ground_truth_files.is_empty() {
        None
    } else {
        Some(compute_ground_truth_metrics(
            &value,
            ground_truth_files,
            FIRST_CORRECT_FILE_RATE_DEFAULT_K,
        ))
    };
    let mut summary_value = serde_json::json!({
        "command": "session-finish",
        "trace": trace.display().to_string(),
        "summary": summary,
        "misses": value.get("misses").cloned().unwrap_or_else(|| serde_json::json!([])),
        "token_accounting": value
            .get("token_accounting")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}))
    });

    if let Some(metrics) = &ground_truth_metrics
        && let Some(obj) = summary_value.as_object_mut()
    {
        obj.insert(
            "first_correct_file_rate_at_k".to_string(),
            serde_json::Value::from(metrics.first_correct_file_rate),
        );
        obj.insert(
            "first_correct_file_rate_k".to_string(),
            serde_json::Value::from(metrics.k as u64),
        );
        obj.insert(
            "turns_to_first_edit".to_string(),
            match metrics.turns_to_first_edit {
                Some(turns) => serde_json::Value::from(turns as u64),
                None => serde_json::Value::Null,
            },
        );
        obj.insert(
            "wrong_files_read".to_string(),
            serde_json::Value::from(metrics.wrong_files_read as u64),
        );
        obj.insert(
            "ground_truth_files".to_string(),
            serde_json::Value::from(
                metrics
                    .ground_truth_files
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        );
    }

    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, serde_json::to_vec_pretty(&summary_value)?)
        .with_context(|| format!("failed to write {}", out.display()))?;

    let (
        first_correct_file_rate_at_k,
        first_correct_file_rate_k,
        turns_to_first_edit,
        wrong_files_read,
    ) = match ground_truth_metrics {
        Some(metrics) => (
            Some(metrics.first_correct_file_rate),
            Some(metrics.k),
            Some(match metrics.turns_to_first_edit {
                Some(turns) => serde_json::Value::from(turns as u64),
                None => serde_json::Value::Null,
            }),
            Some(metrics.wrong_files_read),
        ),
        None => (None, None, None, None),
    };

    Ok(SessionFinishOutput {
        command: "session-finish",
        trace: trace.display().to_string(),
        out: out.display().to_string(),
        summary,
        first_correct_file_rate_at_k,
        first_correct_file_rate_k,
        turns_to_first_edit,
        wrong_files_read,
    })
}

const FIRST_CORRECT_FILE_RATE_DEFAULT_K: usize = 5;

#[derive(Debug, Clone)]
struct GroundTruthMetrics {
    k: usize,
    first_correct_file_rate: f64,
    turns_to_first_edit: Option<usize>,
    wrong_files_read: usize,
    ground_truth_files: BTreeSet<String>,
}

fn compute_ground_truth_metrics(
    trace: &serde_json::Value,
    ground_truth_files: &[String],
    k: usize,
) -> GroundTruthMetrics {
    let ground_truth: BTreeSet<String> = ground_truth_files.iter().cloned().collect();
    let events = trace
        .get("events")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut first_correct_file_rate = 0.0_f64;
    for event in &events {
        let command = event
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let classification = event
            .get("classification")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| classify_session_command(command));
        let is_callsieve_context =
            classification == "callsieve_context" || is_callsieve_context_command_local(command);
        if !is_callsieve_context {
            continue;
        }
        let mut selected = json_string_array(event.get("context_selected_files"));
        if selected.is_empty() {
            selected = json_string_array(event.get("files_read"));
        }
        let top_k: Vec<String> = selected.into_iter().take(k).collect();
        if top_k.iter().any(|file| ground_truth.contains(file)) {
            first_correct_file_rate = 1.0;
        }
        break;
    }

    let mut turns_to_first_edit: Option<usize> = None;
    for (idx, event) in events.iter().enumerate() {
        let command = event
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !is_edit_or_write_command_local(command) {
            continue;
        }
        let targets = json_string_array(event.get("files_read"));
        if targets.iter().any(|file| ground_truth.contains(file)) {
            turns_to_first_edit = Some(idx + 1);
            break;
        }
    }

    let mut wrong_files = BTreeSet::new();
    for event in &events {
        let command = event
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let classification = event
            .get("classification")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| classify_session_command(command));
        let counts_as_full_read = classification == "file_read"
            || is_file_read_command_local(command)
            || is_full_file_grep_with_context_local(command);
        if !counts_as_full_read {
            continue;
        }
        for file in json_string_array(event.get("files_read")) {
            if !ground_truth.contains(&file) {
                wrong_files.insert(file);
            }
        }
    }

    GroundTruthMetrics {
        k,
        first_correct_file_rate,
        turns_to_first_edit,
        wrong_files_read: wrong_files.len(),
        ground_truth_files: ground_truth,
    }
}

fn is_edit_or_write_command_local(command: &str) -> bool {
    if command.is_empty() {
        return false;
    }
    let first = command.split_whitespace().next().unwrap_or_default();
    let first_lower = first.to_ascii_lowercase();
    let lower = command.to_ascii_lowercase();
    matches!(
        first,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Update"
    ) || matches!(
        first_lower.as_str(),
        "edit"
            | "write"
            | "multiedit"
            | "notebookedit"
            | "edit_file"
            | "write_file"
            | "apply_patch"
            | "applypatch"
            | "str_replace"
            | "str_replace_editor"
            | "create_file"
    ) || lower.starts_with("apply_patch ")
        || lower.contains(" edit_file ")
        || lower.contains(" write_file ")
        || lower.contains(" apply_patch ")
}

fn is_full_file_grep_with_context_local(command: &str) -> bool {
    if !is_grep_command_local(command) {
        return false;
    }
    let lower = command.to_ascii_lowercase();
    lower.contains(" -c ")
        || lower.contains(" --context")
        || lower.contains(" -a ")
        || lower.contains(" -b ")
}

fn pilot_init(manifest: &Path, sessions: usize) -> Result<PilotInitOutput> {
    if manifest.exists() {
        anyhow::bail!(
            "refusing to overwrite {}; remove it first or choose another manifest",
            manifest.display()
        );
    }
    if let Some(parent) = manifest
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let strict_claim = sessions >= 100;
    let minimum_planned_tasks = if strict_claim {
        sessions.saturating_mul(12).div_ceil(10)
    } else {
        sessions
    };
    let mut protocol = default_pilot_protocol();
    protocol.minimum_planned_tasks = minimum_planned_tasks;
    let manifest_value = PilotHarnessManifest {
        schema_version: 1,
        target_sessions: sessions,
        protocol,
        thresholds: serde_json::json!({
            "minimum_recall": 1.0,
            "minimum_token_reduction_percent": 0.0,
            "minimum_observed_sessions": sessions,
            "minimum_observed_token_reduction_percent": 50.0,
            "minimum_external_repos": if strict_claim { 6 } else { 0 },
            "minimum_planned_tasks": minimum_planned_tasks,
            "maximum_controlled_replay_ratio": 0.0,
            "maximum_trace_violations": 0,
            "maximum_critical_misses": 0,
            "require_fresh_index": true,
            "require_lsp_where_available": strict_claim,
            "require_codex_bootstrap": strict_claim,
            "require_transcript_token_accounting": strict_claim
        }),
        tasks: Vec::new(),
        rejected_sessions: Vec::new(),
    };
    write_pilot_manifest(manifest, &manifest_value)?;

    Ok(PilotInitOutput {
        command: "pilot-init",
        manifest: manifest.display().to_string(),
        target_sessions: sessions,
        status: "created".to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
fn pilot_task_add(
    manifest_path: &Path,
    repo: &Path,
    task: &str,
    id: Option<String>,
    expected_files: Vec<String>,
    critical_files: Vec<String>,
    external: bool,
    client: AgentClient,
    model: &str,
    suite_path: Option<PathBuf>,
    pair_id: Option<String>,
    task_category: String,
    difficulty: String,
    condition: String,
    token_accounting_source: String,
) -> Result<PilotTaskAddOutput> {
    let mut manifest = read_pilot_manifest(manifest_path)?;
    let id = id.unwrap_or_else(|| next_pilot_task_id(&manifest));
    if manifest.tasks.iter().any(|task| task.id == id) {
        anyhow::bail!("pilot task id already exists: {id}");
    }
    let critical_files = if critical_files.is_empty() {
        expected_files.clone()
    } else {
        critical_files
    };
    let task_dir = pilot_artifact_root(manifest_path).join("tasks").join(&id);
    let trace_path = task_dir.join("combined-observed.json");
    let baseline_trace_path = task_dir.join("baseline-observed.json");
    let callsieve_trace_path = task_dir.join("callsieve-observed.json");
    let summary_path = task_dir.join("summary.json");
    let pair_id = pair_id.unwrap_or_else(|| id.clone());
    let task_entry = PilotHarnessTask {
        id,
        repo: repo.display().to_string(),
        task: task.to_string(),
        client: agent_client_name(client).to_string(),
        model: model.to_string(),
        expected_files,
        critical_files,
        external,
        suite_path: suite_path.map(|path| path.display().to_string()),
        pair_id: Some(pair_id),
        task_category,
        difficulty,
        condition,
        token_accounting_source,
        preregistered: true,
        baseline_trace_path: baseline_trace_path.display().to_string(),
        callsieve_trace_path: callsieve_trace_path.display().to_string(),
        trace_path: trace_path.display().to_string(),
        summary_path: summary_path.display().to_string(),
        status: "pending".to_string(),
    };
    manifest.tasks.push(task_entry.clone());
    write_pilot_manifest(manifest_path, &manifest)?;

    Ok(PilotTaskAddOutput {
        command: "pilot-task add",
        manifest: manifest_path.display().to_string(),
        task: task_entry,
    })
}

fn pilot_task_reject(
    manifest_path: &Path,
    task_id: &str,
    reason: &str,
) -> Result<PilotTaskRejectOutput> {
    if reason.trim().is_empty() {
        anyhow::bail!("rejection reason cannot be empty");
    }
    let mut manifest = read_pilot_manifest(manifest_path)?;
    let index = manifest
        .tasks
        .iter()
        .position(|task| task.id == task_id)
        .with_context(|| format!("pilot task not found: {task_id}"))?;
    let status_at_rejection = manifest.tasks[index].status.clone();
    let trace_path = manifest.tasks[index].trace_path.clone();
    manifest.tasks[index].status = "rejected".to_string();
    let rejected = PilotRejectedSession {
        task_id: task_id.to_string(),
        reason: reason.trim().to_string(),
        status_at_rejection,
        trace_path,
        rejected_at: now_unix_seconds(),
    };
    manifest
        .rejected_sessions
        .retain(|entry| entry.task_id != task_id);
    manifest.rejected_sessions.push(rejected.clone());
    write_pilot_manifest(manifest_path, &manifest)?;

    Ok(PilotTaskRejectOutput {
        command: "pilot-task reject",
        manifest: manifest_path.display().to_string(),
        rejected,
    })
}

fn pilot_run(
    manifest_path: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    command: &str,
    files_read: Vec<String>,
    context_selected_files: Vec<String>,
    tokens: usize,
) -> Result<PilotRunOutput> {
    pilot_run_with_context_and_token_evidence(
        manifest_path,
        task_id,
        mode,
        command,
        files_read,
        context_selected_files,
        tokens,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn pilot_run_with_context_and_token_evidence(
    manifest_path: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    command: &str,
    files_read: Vec<String>,
    context_selected_files: Vec<String>,
    tokens: usize,
    token_evidence: Option<&serde_json::Value>,
) -> Result<PilotRunOutput> {
    let mut manifest = read_pilot_manifest(manifest_path)?;
    let index = manifest
        .tasks
        .iter()
        .position(|task| task.id == task_id)
        .with_context(|| format!("pilot task not found: {task_id}"))?;
    let task = manifest.tasks[index].clone();
    if task.status == "rejected" {
        anyhow::bail!("pilot task is rejected and cannot be updated: {task_id}");
    }
    ensure_pilot_trace(&task, Path::new(&task.trace_path))?;
    let mode_trace = match mode {
        PilotSessionMode::Baseline => Path::new(&task.baseline_trace_path),
        PilotSessionMode::Callsieve => Path::new(&task.callsieve_trace_path),
    };
    ensure_pilot_trace(&task, mode_trace)?;
    let phase = match mode {
        PilotSessionMode::Baseline => SessionPhase::Baseline,
        PilotSessionMode::Callsieve => SessionPhase::Callsieve,
    };
    session_event_with_token_evidence(
        Path::new(&task.trace_path),
        command,
        files_read.clone(),
        context_selected_files.clone(),
        Some(tokens),
        Some(phase),
        token_evidence,
    )?;
    session_event_with_token_evidence(
        mode_trace,
        command,
        files_read,
        context_selected_files,
        Some(tokens),
        Some(phase),
        token_evidence,
    )?;
    let finish = session_finish(
        Path::new(&task.trace_path),
        Path::new(&task.summary_path),
        &[],
    )?;
    let summary_value = serde_json::to_value(&finish.summary)?;
    manifest.tasks[index].status = pilot_task_status(&summary_value);
    write_pilot_manifest(manifest_path, &manifest)?;

    Ok(PilotRunOutput {
        command: "pilot-run",
        manifest: manifest_path.display().to_string(),
        task_id: task_id.to_string(),
        mode,
        trace: task.trace_path,
        mode_trace: mode_trace.display().to_string(),
        summary: finish.summary,
    })
}

#[allow(clippy::too_many_arguments)]
fn pilot_collect_ollama(
    manifest_path: &Path,
    model: &str,
    limit: usize,
    context_limit: usize,
    snippets_per_file: usize,
    baseline_file_limit: usize,
    baseline_line_limit: usize,
) -> Result<PilotCollectLocalOutput> {
    let manifest = read_pilot_manifest(manifest_path)?;
    let candidates: Vec<PilotHarnessTask> = manifest
        .tasks
        .iter()
        .filter(|task| task.status == "pending" || task.status == "baseline_recorded")
        .filter(|task| pilot_task_matches_ollama_model(task, model))
        .take(limit)
        .cloned()
        .collect();
    let skipped_sessions = manifest
        .tasks
        .iter()
        .filter(|task| !candidates.iter().any(|candidate| candidate.id == task.id))
        .filter(|task| task.status == "complete" || task.status == "rejected")
        .count();
    let mut sessions = Vec::new();

    for task in candidates {
        let session = collect_ollama_task(
            manifest_path,
            &task,
            model,
            context_limit,
            snippets_per_file,
            baseline_file_limit,
            baseline_line_limit,
        )
        .with_context(|| format!("failed to collect Ollama pilot task {}", task.id))?;
        sessions.push(session);
    }

    let qa = pilot_qa(manifest_path)?;
    Ok(PilotCollectLocalOutput {
        command: "pilot-collect-ollama",
        manifest: manifest_path.display().to_string(),
        model: model.to_string(),
        base_url: None,
        requested_sessions: limit,
        collected_sessions: sessions.len(),
        skipped_sessions,
        observed_sessions: qa.observed_sessions,
        qa_status: qa.status,
        sessions,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_ollama_task(
    manifest_path: &Path,
    task: &PilotHarnessTask,
    model: &str,
    context_limit: usize,
    snippets_per_file: usize,
    baseline_file_limit: usize,
    baseline_line_limit: usize,
) -> Result<PilotCollectLocalSessionOutput> {
    let root = Path::new(&task.repo);
    let index = load_or_build_index(root)?;
    let task_dir = Path::new(&task.trace_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(task_dir)
        .with_context(|| format!("failed to create {}", task_dir.display()))?;
    let baseline_artifact = task_dir.join("baseline-ollama-transcript.local.json");
    let callsieve_artifact = task_dir.join("callsieve-ollama-transcript.local.json");
    let mut baseline_tokens = 0;
    let mut baseline_files = 0;

    if task.status == "pending" {
        let baseline_plan = build_baseline_prompt_plan(
            root,
            &index,
            task,
            baseline_file_limit,
            baseline_line_limit,
        )?;
        let baseline_run = run_ollama_verbose(model, &baseline_plan.prompt)
            .with_context(|| format!("ollama baseline failed for {}", task.id))?;
        baseline_tokens = baseline_run.prompt_eval_count;
        baseline_files = baseline_plan.files_read.len();
        write_ollama_artifact(
            &baseline_artifact,
            task,
            model,
            "baseline",
            &baseline_plan,
            &baseline_run,
        )?;
        pilot_run(
            manifest_path,
            &task.id,
            PilotSessionMode::Baseline,
            &baseline_plan.command,
            baseline_plan.files_read,
            baseline_plan.context_selected_files,
            baseline_tokens,
        )?;
    } else if Path::new(&task.trace_path).is_file() {
        let trace_json = fs::read_to_string(&task.trace_path)
            .with_context(|| format!("failed to read trace: {}", task.trace_path))?;
        let summary = query::trace_summary_from_str(&trace_json)?;
        let summary_value = serde_json::to_value(&summary)?;
        baseline_tokens = summary_number(&summary_value, "baseline_tokens");
    }

    let callsieve_plan =
        build_callsieve_prompt_plan(root, &index, task, context_limit, snippets_per_file)?;
    let callsieve_run = run_ollama_verbose(model, &callsieve_plan.prompt)
        .with_context(|| format!("ollama CallSieve phase failed for {}", task.id))?;
    let callsieve_tokens = callsieve_run.prompt_eval_count;
    let callsieve_files = callsieve_plan.context_selected_files.len();
    write_ollama_artifact(
        &callsieve_artifact,
        task,
        model,
        "callsieve",
        &callsieve_plan,
        &callsieve_run,
    )?;
    let output = pilot_run(
        manifest_path,
        &task.id,
        PilotSessionMode::Callsieve,
        &callsieve_plan.command,
        callsieve_plan.files_read,
        callsieve_plan.context_selected_files,
        callsieve_tokens,
    )?;
    let summary_value = serde_json::to_value(&output.summary)?;
    if baseline_tokens == 0 {
        baseline_tokens = summary_number(&summary_value, "baseline_tokens");
    }
    let token_savings = baseline_tokens as isize - callsieve_tokens as isize;
    let token_reduction_percent = if baseline_tokens == 0 {
        0.0
    } else {
        (token_savings as f64 / baseline_tokens as f64) * 100.0
    };

    Ok(PilotCollectLocalSessionOutput {
        task_id: task.id.clone(),
        repo: task.repo.clone(),
        status: pilot_task_status(&summary_value),
        baseline_tokens,
        callsieve_tokens,
        token_reduction_percent,
        baseline_files,
        callsieve_files,
        baseline_artifact: baseline_artifact.display().to_string(),
        callsieve_artifact: callsieve_artifact.display().to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
fn pilot_collect_lm_studio(
    manifest_path: &Path,
    model: &str,
    base_url: &str,
    limit: usize,
    context_limit: usize,
    snippets_per_file: usize,
    baseline_file_limit: usize,
    baseline_line_limit: usize,
    max_tokens: usize,
) -> Result<PilotCollectLocalOutput> {
    let manifest = read_pilot_manifest(manifest_path)?;
    let candidates: Vec<PilotHarnessTask> = manifest
        .tasks
        .iter()
        .filter(|task| task.status == "pending" || task.status == "baseline_recorded")
        .filter(|task| pilot_task_matches_lm_studio_model(task, model))
        .take(limit)
        .cloned()
        .collect();
    let skipped_sessions = manifest
        .tasks
        .iter()
        .filter(|task| !candidates.iter().any(|candidate| candidate.id == task.id))
        .filter(|task| task.status == "complete" || task.status == "rejected")
        .count();
    let mut sessions = Vec::new();

    for task in candidates {
        let session = collect_lm_studio_task(
            manifest_path,
            &task,
            model,
            base_url,
            context_limit,
            snippets_per_file,
            baseline_file_limit,
            baseline_line_limit,
            max_tokens,
        )
        .with_context(|| format!("failed to collect LM Studio pilot task {}", task.id))?;
        sessions.push(session);
    }

    let qa = pilot_qa(manifest_path)?;
    Ok(PilotCollectLocalOutput {
        command: "pilot-collect-lm-studio",
        manifest: manifest_path.display().to_string(),
        model: model.to_string(),
        base_url: Some(base_url.to_string()),
        requested_sessions: limit,
        collected_sessions: sessions.len(),
        skipped_sessions,
        observed_sessions: qa.observed_sessions,
        qa_status: qa.status,
        sessions,
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_lm_studio_task(
    manifest_path: &Path,
    task: &PilotHarnessTask,
    model: &str,
    base_url: &str,
    context_limit: usize,
    snippets_per_file: usize,
    baseline_file_limit: usize,
    baseline_line_limit: usize,
    max_tokens: usize,
) -> Result<PilotCollectLocalSessionOutput> {
    let root = Path::new(&task.repo);
    let index = load_or_build_index(root)?;
    let task_dir = Path::new(&task.trace_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(task_dir)
        .with_context(|| format!("failed to create {}", task_dir.display()))?;
    let baseline_artifact = task_dir.join("baseline-lm-studio-transcript.local.json");
    let callsieve_artifact = task_dir.join("callsieve-lm-studio-transcript.local.json");
    let mut baseline_tokens = 0;
    let mut baseline_files = 0;

    if task.status == "pending" {
        let baseline_plan = build_baseline_prompt_plan(
            root,
            &index,
            task,
            baseline_file_limit,
            baseline_line_limit,
        )?;
        let baseline_run = run_lm_studio_chat(base_url, model, &baseline_plan.prompt, max_tokens)
            .with_context(|| format!("LM Studio baseline failed for {}", task.id))?;
        baseline_tokens = baseline_run.prompt_tokens;
        baseline_files = baseline_plan.files_read.len();
        write_lm_studio_artifact(
            &baseline_artifact,
            task,
            model,
            base_url,
            "baseline",
            &baseline_plan,
            &baseline_run,
        )?;
        pilot_run(
            manifest_path,
            &task.id,
            PilotSessionMode::Baseline,
            &baseline_plan.command,
            baseline_plan.files_read,
            baseline_plan.context_selected_files,
            baseline_tokens,
        )?;
    } else if Path::new(&task.trace_path).is_file() {
        let trace_json = fs::read_to_string(&task.trace_path)
            .with_context(|| format!("failed to read trace: {}", task.trace_path))?;
        let summary = query::trace_summary_from_str(&trace_json)?;
        let summary_value = serde_json::to_value(&summary)?;
        baseline_tokens = summary_number(&summary_value, "baseline_tokens");
    }

    let callsieve_plan =
        build_callsieve_prompt_plan(root, &index, task, context_limit, snippets_per_file)?;
    let callsieve_run = run_lm_studio_chat(base_url, model, &callsieve_plan.prompt, max_tokens)
        .with_context(|| format!("LM Studio CallSieve phase failed for {}", task.id))?;
    let callsieve_tokens = callsieve_run.prompt_tokens;
    let callsieve_files = callsieve_plan.context_selected_files.len();
    write_lm_studio_artifact(
        &callsieve_artifact,
        task,
        model,
        base_url,
        "callsieve",
        &callsieve_plan,
        &callsieve_run,
    )?;
    let output = pilot_run(
        manifest_path,
        &task.id,
        PilotSessionMode::Callsieve,
        &callsieve_plan.command,
        callsieve_plan.files_read,
        callsieve_plan.context_selected_files,
        callsieve_tokens,
    )?;
    let summary_value = serde_json::to_value(&output.summary)?;
    if baseline_tokens == 0 {
        baseline_tokens = summary_number(&summary_value, "baseline_tokens");
    }
    let token_savings = baseline_tokens as isize - callsieve_tokens as isize;
    let token_reduction_percent = if baseline_tokens == 0 {
        0.0
    } else {
        (token_savings as f64 / baseline_tokens as f64) * 100.0
    };

    Ok(PilotCollectLocalSessionOutput {
        task_id: task.id.clone(),
        repo: task.repo.clone(),
        status: pilot_task_status(&summary_value),
        baseline_tokens,
        callsieve_tokens,
        token_reduction_percent,
        baseline_files,
        callsieve_files,
        baseline_artifact: baseline_artifact.display().to_string(),
        callsieve_artifact: callsieve_artifact.display().to_string(),
    })
}

fn pilot_task_matches_ollama_model(task: &PilotHarnessTask, model: &str) -> bool {
    task.model
        .strip_prefix("ollama:")
        .is_some_and(|registered| registered == model)
        || task.model == model
}

fn pilot_task_matches_lm_studio_model(task: &PilotHarnessTask, model: &str) -> bool {
    task.model
        .strip_prefix("lm-studio:")
        .is_some_and(|registered| registered == model)
        || task
            .model
            .strip_prefix("openai-compatible:")
            .is_some_and(|registered| registered == model)
        || task.model == model
        || task.client == "generic"
}

fn load_or_build_index(root: &Path) -> Result<store::CodeIndex> {
    match store::json_store::load_index(root) {
        Ok(index) => Ok(index),
        Err(_) => {
            let index = indexer::build_index(root)?;
            store::json_store::save_index(root, &index)?;
            Ok(index)
        }
    }
}

fn build_baseline_prompt_plan(
    root: &Path,
    index: &store::CodeIndex,
    task: &PilotHarnessTask,
    file_limit: usize,
    line_limit: usize,
) -> Result<PilotPromptPlan> {
    let search_task = task_search_text(&task.task);
    let terms = pilot_search_terms(&search_task);
    let mut evidence = baseline_search_evidence(root, index, &terms)?;
    evidence.sort_by(|left, right| {
        right
            .match_lines
            .len()
            .cmp(&left.match_lines.len())
            .then(left.path.cmp(&right.path))
    });
    let selected: Vec<FileSearchEvidence> = evidence.into_iter().take(file_limit).collect();
    let files_read: Vec<String> = selected.iter().map(|file| file.path.clone()).collect();
    let mut result_lines = Vec::new();
    for file in &selected {
        for line in &file.result_lines {
            if result_lines.len() >= line_limit {
                break;
            }
            result_lines.push(line.clone());
        }
        if result_lines.len() >= line_limit {
            break;
        }
    }
    let command = format!("rg -n {:?} {}", terms.join("|"), root.display());
    let mut prompt = String::new();
    prompt.push_str("You are an audited local coding agent baseline phase.\n");
    prompt.push_str("Do not use CallSieve in this phase. Use only the raw rg-style evidence and file snippets below.\n");
    prompt.push_str("Return compact JSON with likely_files and a one-sentence rationale.\n\n");
    prompt.push_str(&format!("TASK: {}\n", task.task));
    prompt.push_str(&format!("PRIMARY_TASK: {search_task}\n"));
    prompt.push_str(&format!("REPO: {}\n", root.display()));
    prompt.push_str(&format!("COMMAND: {command}\n"));
    prompt.push_str(&format!("SEARCH_TERMS: {}\n", terms.join(", ")));
    prompt.push_str("AUDITED_FILES:\n");
    for file in &files_read {
        prompt.push_str("- ");
        prompt.push_str(file);
        prompt.push('\n');
    }
    prompt.push_str("\nRG_OUTPUT:\n");
    for line in &result_lines {
        prompt.push_str(line);
        prompt.push('\n');
    }
    prompt.push_str("\nFILE_SNIPPETS:\n");
    for file in &selected {
        prompt.push_str(&format!("--- {} ---\n", file.path));
        prompt.push_str(&snippet_for_match_lines(
            &file.content,
            &file.match_lines,
            2,
            28,
        ));
        prompt.push('\n');
    }
    prompt.push_str("\nReturn JSON only.\n");

    Ok(PilotPromptPlan {
        command,
        files_read,
        context_selected_files: Vec::new(),
        prompt,
    })
}

fn build_callsieve_prompt_plan(
    root: &Path,
    index: &store::CodeIndex,
    task: &PilotHarnessTask,
    context_limit: usize,
    snippets_per_file: usize,
) -> Result<PilotPromptPlan> {
    let context = query::build_context(
        root,
        index,
        &task.task,
        context_limit,
        snippets_per_file,
        true,
    )?;
    let context_value = serde_json::to_value(&context)?;
    let context_selected_files = context_read_first_files(&context_value);
    let compact_context = compact_agent_context_value(&context_value);
    let command = format!(
        "callsieve agent-context {} {:?} --limit {context_limit} --snippets-per-file {snippets_per_file}",
        root.display(),
        task.task
    );
    let mut prompt = String::new();
    prompt.push_str("You are an audited local coding agent CallSieve phase.\n");
    prompt.push_str("Use the CallSieve read-first context below before any broad search.\n");
    prompt.push_str("If the context is sufficient, answer from it without reading whole files. Use Read or Grep only for missing evidence.\n");
    prompt.push_str("Return compact JSON with context_selected_files copied from the read_first packet, files_read containing only actual Read tool paths, would_change, needed_more_context, and a one-sentence rationale.\n\n");
    prompt.push_str(&format!("TASK: {}\n", task.task));
    prompt.push_str(&format!("REPO: {}\n", root.display()));
    prompt.push_str(&format!("COMMAND: {command}\n"));
    prompt.push_str("CONTEXT_SELECTED_FILES:\n");
    for file in &context_selected_files {
        prompt.push_str("- ");
        prompt.push_str(file);
        prompt.push('\n');
    }
    prompt.push_str("\nCALLSIEVE_AGENT_CONTEXT_JSON:\n");
    prompt.push_str(&serde_json::to_string_pretty(&compact_context)?);
    prompt.push_str("\n\nReturn JSON only.\n");

    Ok(PilotPromptPlan {
        command,
        files_read: Vec::new(),
        context_selected_files,
        prompt,
    })
}

fn baseline_search_evidence(
    root: &Path,
    index: &store::CodeIndex,
    terms: &[String],
) -> Result<Vec<FileSearchEvidence>> {
    let mut files = Vec::new();
    for file in &index.files {
        let content = match fs::read_to_string(root.join(&file.path)) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let mut match_lines = Vec::new();
        let mut result_lines = Vec::new();
        for (line_index, line) in content.lines().enumerate() {
            let line_lower = line.to_ascii_lowercase();
            if terms.iter().any(|term| line_lower.contains(term)) {
                let line_number = line_index + 1;
                match_lines.push(line_number);
                result_lines.push(format!("{}:{}:{}", file.path, line_number, line.trim()));
            }
        }
        if !match_lines.is_empty() {
            files.push(FileSearchEvidence {
                path: file.path.clone(),
                content,
                match_lines,
                result_lines,
            });
        }
    }
    Ok(files)
}

fn snippet_for_match_lines(
    content: &str,
    match_lines: &[usize],
    radius: usize,
    max_lines: usize,
) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut selected = BTreeSet::new();
    for line in match_lines {
        let start = line.saturating_sub(radius + 1);
        let end = (*line + radius).min(lines.len());
        for index in start..end {
            selected.insert(index);
            if selected.len() >= max_lines {
                break;
            }
        }
        if selected.len() >= max_lines {
            break;
        }
    }
    let mut snippet = String::new();
    let mut previous = None;
    for index in selected {
        if previous.is_some_and(|last: usize| index > last + 1) {
            snippet.push_str("...\n");
        }
        snippet.push_str(&format!("{:>5}: {}\n", index + 1, lines[index]));
        previous = Some(index);
    }
    snippet
}

fn task_search_text(task: &str) -> String {
    task.split(';').next().unwrap_or(task).trim().to_string()
}

fn pilot_search_terms(task: &str) -> Vec<String> {
    let stopwords = BTreeSet::from([
        "about",
        "and",
        "behavior",
        "change",
        "code",
        "file",
        "find",
        "fix",
        "for",
        "from",
        "handling",
        "how",
        "implement",
        "make",
        "observed",
        "ollama",
        "round",
        "the",
        "this",
        "update",
        "what",
        "where",
        "with",
    ]);
    let mut terms = Vec::new();
    let mut current = String::new();
    for character in task.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            current.push(character.to_ascii_lowercase());
        } else if !current.is_empty() {
            if current.len() >= 3 && !stopwords.contains(current.as_str()) {
                terms.push(current.clone());
            }
            current.clear();
        }
    }
    if !current.is_empty() && current.len() >= 3 && !stopwords.contains(current.as_str()) {
        terms.push(current);
    }
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        terms.push(task.to_ascii_lowercase());
    }
    terms
}

fn context_read_first_files(context_value: &serde_json::Value) -> Vec<String> {
    context_value
        .get("read_first")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            file.get("f")
                .or_else(|| file.get("file"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_string)
        .collect()
}

fn context_read_first_symbol_names(context_value: &serde_json::Value) -> BTreeSet<String> {
    context_value
        .get("read_first")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| json_array_any(file, &["sy", "symbols"]))
        .flatten()
        .filter_map(symbol_name_from_value)
        .map(str::to_string)
        .collect()
}

fn context_value_read_first_targets(context_value: &serde_json::Value) -> Vec<query::FocusTarget> {
    context_value
        .get("read_first")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            let path = file
                .get("f")
                .or_else(|| file.get("file"))
                .and_then(serde_json::Value::as_str)?;
            let selected_symbol = file
                .get("sy")
                .or_else(|| file.get("symbols"))
                .and_then(serde_json::Value::as_array)
                .and_then(|symbols| {
                    symbols.iter().find(|symbol| {
                        let kind = symbol_kind_from_value(symbol).unwrap_or_default();
                        !matches!(
                            kind,
                            "macro" | "call" | "reference" | "import" | "use" | "include"
                        )
                    })
                });
            let symbol = selected_symbol
                .and_then(symbol_name_from_value)
                .map(str::to_string);
            let line = selected_symbol.and_then(symbol_line_from_value);
            Some(query::FocusTarget {
                file: path.to_string(),
                symbol,
                line,
                is_code: indexer::language::Language::from_path(Path::new(path))
                    .map(indexer::language::Language::is_code)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn symbol_line_from_value(symbol: &serde_json::Value) -> Option<usize> {
    symbol
        .get("l")
        .or_else(|| symbol.get("line"))
        .and_then(serde_json::Value::as_u64)
        .and_then(|line| usize::try_from(line).ok())
        .or_else(|| {
            symbol
                .get("ls")
                .or_else(|| symbol.get("lines"))
                .and_then(serde_json::Value::as_array)
                .and_then(|lines| lines.first())
                .and_then(serde_json::Value::as_u64)
                .and_then(|line| usize::try_from(line).ok())
        })
        .or_else(|| {
            symbol
                .as_array()
                .and_then(|items| items.get(1))
                .and_then(serde_json::Value::as_u64)
                .and_then(|line| usize::try_from(line).ok())
        })
}

fn compact_agent_context_value(context: &serde_json::Value) -> serde_json::Value {
    let read_first = context
        .get("read_first")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(compact_context_file_value)
        .collect::<Vec<_>>();
    serde_json::json!({
        "task": context.get("task").cloned().unwrap_or(serde_json::Value::Null),
        "root": context.get("root").cloned().unwrap_or(serde_json::Value::Null),
        "read_first": read_first
    })
}

fn compact_context_file_value(file: &serde_json::Value) -> serde_json::Value {
    let symbols = file
        .get("sy")
        .or_else(|| file.get("symbols"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(6)
        .map(|symbol| {
            let mut compact = serde_json::Map::new();
            if let Some(name) = symbol_name_from_value(symbol) {
                compact.insert("name".to_string(), serde_json::json!(name));
            }
            if let Some(kind) = symbol_kind_from_value(symbol) {
                compact.insert("kind".to_string(), serde_json::json!(kind));
            }
            if let Some(lines) = symbol.get("ls").or_else(|| symbol.get("lines")) {
                compact.insert("lines".to_string(), lines.clone());
            } else if let Some(line) = symbol.get("l").or_else(|| symbol.get("line")) {
                compact.insert("line".to_string(), line.clone());
            } else if let Some(items) = symbol.as_array()
                && let (Some(start), Some(end)) = (
                    items.get(1).and_then(serde_json::Value::as_u64),
                    items.get(2).and_then(serde_json::Value::as_u64),
                )
            {
                compact.insert("lines".to_string(), serde_json::json!([start, end]));
            } else if let Some(line) = symbol_line_from_value(symbol) {
                compact.insert("line".to_string(), serde_json::json!(line));
            }
            if let Some(signature) = symbol.get("signature") {
                compact.insert("signature".to_string(), signature.clone());
            }
            serde_json::Value::Object(compact)
        })
        .collect::<Vec<_>>();
    let snippets = file
        .get("snippets")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(1)
        .cloned()
        .collect::<Vec<_>>();
    let related_tests = file
        .get("related_tests")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .map(|test| {
            serde_json::json!({
                "file": test.get("file").cloned().unwrap_or(serde_json::Value::Null)
            })
        })
        .collect::<Vec<_>>();
    let why = file
        .get("w")
        .or_else(|| file.get("why"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .cloned()
        .collect::<Vec<_>>();
    serde_json::json!({
        "rank": file.get("rank").cloned().unwrap_or(serde_json::Value::Null),
        "score": file.get("s").or_else(|| file.get("score")).cloned().unwrap_or(serde_json::Value::Null),
        "file": file.get("f").or_else(|| file.get("file")).cloned().unwrap_or(serde_json::Value::Null),
        "risk": context_file_risk_value(file).unwrap_or(serde_json::Value::Null),
        "symbols": symbols,
        "snippets": snippets,
        "related_tests": related_tests,
        "why": why
    })
}

fn compact_or_legacy_impact_tests_len(impact: &serde_json::Value) -> Option<usize> {
    if let Some(tests) = impact.get("t").or_else(|| impact.get("tests")) {
        return tests
            .as_array()
            .map(Vec::len)
            .or_else(|| tests.as_str().map(|_| 1));
    }
    let items = impact.as_array()?;
    let test_value = items.get(1)?;
    if test_value.is_string() {
        Some(1)
    } else {
        test_value.as_array().map(Vec::len)
    }
}

fn context_file_risk_value(file: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(impact) = file.get("i").or_else(|| file.get("impact")) {
        if let Some(risk) = impact.get("r").or_else(|| impact.get("risk")) {
            return Some(risk.clone());
        }
        if let Some(risk) = impact.as_array().and_then(|items| items.first()) {
            return Some(risk.clone());
        }
    }
    file.get("blast_radius")
        .and_then(|blast_radius| blast_radius.get("risk"))
        .cloned()
}

fn write_ollama_artifact(
    path: &Path,
    task: &PilotHarnessTask,
    model: &str,
    phase: &str,
    plan: &PilotPromptPlan,
    run: &OllamaRun,
) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let artifact = OllamaTranscriptArtifact {
        schema_version: 1,
        collection: "observed_session",
        collector: "callsieve pilot-collect-ollama",
        task_id: task.id.clone(),
        phase: phase.to_string(),
        repo: task.repo.clone(),
        model: model.to_string(),
        command: plan.command.clone(),
        files_read: plan.files_read.clone(),
        context_selected_files: plan.context_selected_files.clone(),
        prompt: plan.prompt.clone(),
        response: run.response.clone(),
        token_accounting: OllamaTokenAccounting {
            source: "ollama_verbose_prompt_eval_count",
            counted_tokens: run.prompt_eval_count,
            prompt_eval_count: run.prompt_eval_count,
            eval_count: run.eval_count,
        },
        created_at: now_unix_seconds(),
    };
    fs::write(path, serde_json::to_vec_pretty(&artifact)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn write_lm_studio_artifact(
    path: &Path,
    task: &PilotHarnessTask,
    model: &str,
    base_url: &str,
    phase: &str,
    plan: &PilotPromptPlan,
    run: &LmStudioRun,
) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let artifact = LmStudioTranscriptArtifact {
        schema_version: 1,
        collection: "observed_session",
        collector: "callsieve pilot-collect-lm-studio",
        task_id: task.id.clone(),
        phase: phase.to_string(),
        repo: task.repo.clone(),
        model: model.to_string(),
        base_url: base_url.to_string(),
        command: plan.command.clone(),
        files_read: plan.files_read.clone(),
        context_selected_files: plan.context_selected_files.clone(),
        prompt: plan.prompt.clone(),
        response: run.response.clone(),
        token_accounting: LmStudioTokenAccounting {
            source: "lm_studio_openai_usage_prompt_tokens",
            counted_tokens: run.prompt_tokens,
            prompt_tokens: run.prompt_tokens,
            completion_tokens: run.completion_tokens,
            total_tokens: run.total_tokens,
        },
        raw_response: run.raw_response.clone(),
        created_at: now_unix_seconds(),
    };
    fs::write(path, serde_json::to_vec_pretty(&artifact)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn run_ollama_verbose(model: &str, prompt: &str) -> Result<OllamaRun> {
    let mut command = ProcessCommand::new("ollama");
    command
        .arg("run")
        .arg(model)
        .arg("--verbose")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| "failed to start ollama; verify `ollama list` works")?;
    {
        let mut stdin = child.stdin.take().context("failed to open ollama stdin")?;
        stdin
            .write_all(prompt.as_bytes())
            .context("failed to write prompt to ollama stdin")?;
    }
    let output = child
        .wait_with_output()
        .context("failed to wait for ollama")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let raw = format!("{stdout}\n{stderr}");
    let clean = strip_ansi(&raw);
    if !output.status.success() {
        anyhow::bail!("ollama exited with status {}\n{clean}", output.status);
    }
    let (prompt_eval_count, eval_count) = parse_ollama_verbose_counts(&clean)?;
    let response = clean
        .split("total duration:")
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    Ok(OllamaRun {
        response,
        prompt_eval_count,
        eval_count,
    })
}

fn run_lm_studio_chat(
    base_url: &str,
    model: &str,
    prompt: &str,
    max_tokens: usize,
) -> Result<LmStudioRun> {
    let endpoint = openai_chat_endpoint(base_url)?;
    let request = serde_json::json!({
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": prompt
            }
        ],
        "temperature": 0.0,
        "stream": false,
        "max_tokens": max_tokens
    });
    let response_body = post_json_http(&endpoint, &request)
        .with_context(|| format!("failed to call LM Studio at {base_url}"))?;
    let raw_response: serde_json::Value =
        serde_json::from_str(&response_body).with_context(|| "LM Studio returned invalid JSON")?;
    let prompt_tokens = json_usize(&raw_response, &["usage", "prompt_tokens"])
        .context("LM Studio response missing usage.prompt_tokens")?;
    let completion_tokens =
        json_usize(&raw_response, &["usage", "completion_tokens"]).unwrap_or_default();
    let total_tokens = json_usize(&raw_response, &["usage", "total_tokens"])
        .unwrap_or(prompt_tokens + completion_tokens);
    let response = raw_response
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| {
            choice
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| choice.get("text").and_then(serde_json::Value::as_str))
        })
        .unwrap_or_default()
        .to_string();

    Ok(LmStudioRun {
        response,
        prompt_tokens,
        completion_tokens,
        total_tokens,
        raw_response,
    })
}

fn openai_chat_endpoint(base_url: &str) -> Result<HttpEndpoint> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("http://")
        .context("only http:// LM Studio endpoints are supported")?;
    let (authority, raw_path) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.trim().is_empty() {
        anyhow::bail!("LM Studio base URL is missing a host");
    }
    let (host, port) = parse_http_authority(authority)?;
    let mut path = if raw_path.trim().is_empty() {
        "/v1".to_string()
    } else {
        format!("/{}", raw_path.trim_matches('/'))
    };
    if !path.ends_with("/chat/completions") {
        path.push_str("/chat/completions");
    }
    Ok(HttpEndpoint { host, port, path })
}

fn parse_http_authority(authority: &str) -> Result<(String, u16)> {
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && !port.is_empty() => {
            let port = port
                .parse::<u16>()
                .with_context(|| format!("invalid LM Studio port: {port}"))?;
            (host.to_string(), port)
        }
        _ => (authority.to_string(), 80),
    };
    Ok((host, port))
}

fn post_json_http(endpoint: &HttpEndpoint, body: &serde_json::Value) -> Result<String> {
    let body = serde_json::to_string(body)?;
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .with_context(|| format!("failed to connect to {}:{}", endpoint.host, endpoint.port))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(900)))
        .context("failed to set LM Studio read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .context("failed to set LM Studio write timeout")?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.path,
        endpoint.host,
        endpoint.port,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .context("failed to write LM Studio request")?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .context("failed to read LM Studio response")?;
    let (status, body) = parse_http_response(&response)?;
    if !(200..300).contains(&status) {
        anyhow::bail!("LM Studio returned HTTP {status}: {body}");
    }
    Ok(body)
}

fn parse_http_response(response: &[u8]) -> Result<(u16, String)> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .context("HTTP response missing header terminator")?;
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .context("HTTP response missing numeric status")?;
    let body = &response[header_end + 4..];
    let body = if headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"))
    {
        decode_chunked_body(body)?
    } else {
        body.to_vec()
    };
    let body = String::from_utf8(body).context("HTTP response body was not UTF-8")?;
    Ok((status, body))
}

fn decode_chunked_body(mut input: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    loop {
        let line_end = find_crlf(input).context("chunked body missing chunk size")?;
        let size_text = String::from_utf8_lossy(&input[..line_end]);
        let size = usize::from_str_radix(size_text.split_whitespace().next().unwrap_or(""), 16)
            .with_context(|| format!("invalid chunk size: {size_text}"))?;
        input = &input[line_end + 2..];
        if size == 0 {
            break;
        }
        if input.len() < size + 2 {
            anyhow::bail!("chunked body ended before full chunk");
        }
        output.extend_from_slice(&input[..size]);
        input = &input[size + 2..];
    }
    Ok(output)
}

fn find_crlf(input: &[u8]) -> Option<usize> {
    input.windows(2).position(|window| window == b"\r\n")
}

fn parse_ollama_verbose_counts(output: &str) -> Result<(usize, usize)> {
    let prompt_eval_count = extract_ollama_count(output, "prompt eval count:")
        .context("ollama verbose output did not include prompt eval count")?;
    let eval_count = extract_ollama_count(output, "eval count:")
        .context("ollama verbose output missing eval count")?;
    Ok((prompt_eval_count, eval_count))
}

fn extract_ollama_count(output: &str, label: &str) -> Option<usize> {
    output.lines().find_map(|line| {
        let line = line.trim_start();
        let rest = line.strip_prefix(label)?.trim();
        let number = rest.split_whitespace().next()?;
        number.parse().ok()
    })
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        if character != '\r' {
            output.push(character);
        }
    }
    output
}

fn count_countable_pilot_sessions(
    manifest: &PilotHarnessManifest,
    require_transcript_token_accounting: bool,
) -> Result<usize> {
    let mut count = 0;
    for task in &manifest.tasks {
        if task.status == "rejected" || !Path::new(&task.trace_path).is_file() {
            continue;
        }
        if pilot_task_is_countable(task, require_transcript_token_accounting)? {
            count += 1;
        }
    }
    Ok(count)
}

fn pilot_task_is_countable(
    task: &PilotHarnessTask,
    require_transcript_token_accounting: bool,
) -> Result<bool> {
    let pair_id = task.pair_id.as_deref().unwrap_or(&task.id);
    let preregistered = task.preregistered
        && !task.task.trim().is_empty()
        && !pair_id.trim().is_empty()
        && !task.task_category.trim().is_empty()
        && !task.condition.trim().is_empty()
        && !task.expected_files.is_empty()
        && !task.critical_files.is_empty();
    let registered_token_source = if require_transcript_token_accounting {
        task.token_accounting_source == "transcript_context_tokens"
    } else {
        !task.token_accounting_source.trim().is_empty()
    };
    let trace_path = Path::new(&task.trace_path);
    let trace_json = fs::read_to_string(trace_path)
        .with_context(|| format!("failed to read trace: {}", trace_path.display()))?;
    let trace_value: serde_json::Value = serde_json::from_str(&trace_json)
        .with_context(|| format!("failed to parse trace: {}", trace_path.display()))?;
    let summary = query::trace_summary_from_str(&trace_json)?;
    let summary_value = serde_json::to_value(&summary)?;
    let task_text_matches = trace_value
        .get("task")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|trace_task| trace_task == task.task);
    let observed = summary_number(&summary_value, "observed_sessions");
    let controlled = summary_number(&summary_value, "controlled_replay_sessions");
    let baseline_tokens = summary_number(&summary_value, "baseline_tokens");
    let callsieve_tokens = summary_number(&summary_value, "callsieve_tokens");
    let critical_misses = summary_number(&summary_value, "critical_files_still_missed");
    let complete_metrics =
        observed == 1 && controlled == 0 && baseline_tokens > 0 && callsieve_tokens > 0;
    let violations = if Path::new(&task.callsieve_trace_path).is_file() {
        let callsieve_trace_json = fs::read_to_string(&task.callsieve_trace_path)
            .with_context(|| format!("failed to read trace: {}", task.callsieve_trace_path))?;
        let policy = query::trace_check_from_str_with_options(&callsieve_trace_json, true)?;
        let policy_value = serde_json::to_value(policy)?;
        summary_number(&policy_value, "violations")
    } else {
        1
    };
    let observed_collection = trace_value
        .get("metadata")
        .and_then(|metadata| metadata.get("collection"))
        .and_then(serde_json::Value::as_str)
        == Some("observed_session");
    let trace_token_source = trace_token_accounting_source(&trace_value);
    let transcript_token_accounting = if require_transcript_token_accounting {
        trace_token_source == "transcript_context_tokens"
    } else {
        !trace_token_source.is_empty()
    };

    Ok(preregistered
        && registered_token_source
        && task_text_matches
        && complete_metrics
        && critical_misses == 0
        && violations == 0
        && observed_collection
        && !trace_has_controlled_replay_marker(&trace_json)
        && !trace_has_hook_trace_marker(&trace_json)
        && transcript_token_accounting)
}

fn pilot_qa(manifest_path: &Path) -> Result<PilotQaOutput> {
    let manifest = read_pilot_manifest(manifest_path)?;
    let mut results = Vec::new();
    let mut external_repos = BTreeSet::new();
    let minimum_planned_tasks = threshold_number(&manifest.thresholds, "minimum_planned_tasks")
        .max(manifest.protocol.minimum_planned_tasks);
    let minimum_external_repos = threshold_number(&manifest.thresholds, "minimum_external_repos");
    let require_transcript_token_accounting =
        threshold_bool(&manifest.thresholds, "require_transcript_token_accounting");
    let complete_observed_sessions =
        count_countable_pilot_sessions(&manifest, require_transcript_token_accounting)?;
    let allow_uncollected_buffer_tasks = complete_observed_sessions >= manifest.target_sessions;

    for task in &manifest.tasks {
        if task.status == "rejected" {
            let rejected = manifest
                .rejected_sessions
                .iter()
                .find(|entry| entry.task_id == task.id);
            let audited = rejected.is_some_and(|entry| !entry.reason.trim().is_empty());
            push_qa(
                &mut results,
                &task.id,
                "rejected_session_audit",
                audited,
                "rejected session has an audit reason".to_string(),
                "rejected session is missing an audit reason".to_string(),
            );
            continue;
        }

        if task.external {
            external_repos.insert(task.repo.clone());
        }
        let pair_id = task.pair_id.as_deref().unwrap_or(&task.id);
        let preregistered = task.preregistered
            && !task.task.trim().is_empty()
            && !pair_id.trim().is_empty()
            && !task.task_category.trim().is_empty()
            && !task.condition.trim().is_empty()
            && !task.expected_files.is_empty()
            && !task.critical_files.is_empty();
        push_qa(
            &mut results,
            &task.id,
            "pre_registered_task",
            preregistered,
            "task has frozen pre-registration metadata".to_string(),
            "task is missing pre-registration metadata, expected files, or critical files"
                .to_string(),
        );
        let registered_token_source = if require_transcript_token_accounting {
            task.token_accounting_source == "transcript_context_tokens"
        } else {
            !task.token_accounting_source.trim().is_empty()
        };
        push_qa(
            &mut results,
            &task.id,
            "registered_token_source",
            registered_token_source,
            "task token source is registered".to_string(),
            "task token source is missing or not transcript_context_tokens".to_string(),
        );

        let trace_path = Path::new(&task.trace_path);
        let trace_exists = trace_path.is_file();
        let baseline_trace_exists = Path::new(&task.baseline_trace_path).is_file();
        let callsieve_trace_exists = Path::new(&task.callsieve_trace_path).is_file();
        let uncollected_buffer_task = allow_uncollected_buffer_tasks
            && !trace_exists
            && !baseline_trace_exists
            && !callsieve_trace_exists;
        push_qa(
            &mut results,
            &task.id,
            "combined_trace_exists",
            trace_exists || uncollected_buffer_task,
            if uncollected_buffer_task {
                "uncollected planned buffer task is allowed after target sessions are met"
                    .to_string()
            } else {
                format!("combined trace exists at {}", task.trace_path)
            },
            if uncollected_buffer_task {
                "uncollected planned buffer task is allowed after target sessions are met"
                    .to_string()
            } else {
                format!("combined trace is missing at {}", task.trace_path)
            },
        );
        push_qa(
            &mut results,
            &task.id,
            "baseline_trace_exists",
            baseline_trace_exists || uncollected_buffer_task,
            if uncollected_buffer_task {
                "uncollected planned buffer task is allowed after target sessions are met"
                    .to_string()
            } else {
                format!("baseline trace exists at {}", task.baseline_trace_path)
            },
            if uncollected_buffer_task {
                "uncollected planned buffer task is allowed after target sessions are met"
                    .to_string()
            } else {
                format!("baseline trace is missing at {}", task.baseline_trace_path)
            },
        );
        push_qa(
            &mut results,
            &task.id,
            "callsieve_trace_exists",
            callsieve_trace_exists || uncollected_buffer_task,
            if uncollected_buffer_task {
                "uncollected planned buffer task is allowed after target sessions are met"
                    .to_string()
            } else {
                format!("CallSieve trace exists at {}", task.callsieve_trace_path)
            },
            if uncollected_buffer_task {
                "uncollected planned buffer task is allowed after target sessions are met"
                    .to_string()
            } else {
                format!(
                    "CallSieve trace is missing at {}",
                    task.callsieve_trace_path
                )
            },
        );
        if !trace_exists {
            continue;
        }

        let trace_json = fs::read_to_string(trace_path)
            .with_context(|| format!("failed to read trace: {}", trace_path.display()))?;
        let trace_value: serde_json::Value = serde_json::from_str(&trace_json)
            .with_context(|| format!("failed to parse trace: {}", trace_path.display()))?;
        let summary = query::trace_summary_from_str(&trace_json)?;
        let summary_value = serde_json::to_value(&summary)?;
        let task_text_matches = trace_value
            .get("task")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|trace_task| trace_task == task.task);
        push_qa(
            &mut results,
            &task.id,
            "task_text_matches",
            task_text_matches,
            "trace task text matches manifest task".to_string(),
            "trace task text does not match manifest task".to_string(),
        );
        let observed = summary_number(&summary_value, "observed_sessions");
        let controlled = summary_number(&summary_value, "controlled_replay_sessions");
        let baseline_tokens = summary_number(&summary_value, "baseline_tokens");
        let callsieve_tokens = summary_number(&summary_value, "callsieve_tokens");
        let critical_misses = summary_number(&summary_value, "critical_files_still_missed");
        let complete_metrics =
            observed == 1 && controlled == 0 && baseline_tokens > 0 && callsieve_tokens > 0;
        push_qa(
            &mut results,
            &task.id,
            "paired_observed_session",
            complete_metrics,
            "paired observed baseline and CallSieve phases are complete".to_string(),
            format!(
                "expected one observed pair with baseline and CallSieve tokens; observed={observed}, controlled={controlled}, baseline_tokens={baseline_tokens}, callsieve_tokens={callsieve_tokens}"
            ),
        );
        push_qa(
            &mut results,
            &task.id,
            "critical_misses",
            critical_misses == 0,
            "critical files were selected/read by CallSieve".to_string(),
            format!("critical misses: {critical_misses}"),
        );
        let violations = if Path::new(&task.callsieve_trace_path).is_file() {
            let callsieve_trace_json = fs::read_to_string(&task.callsieve_trace_path)
                .with_context(|| format!("failed to read trace: {}", task.callsieve_trace_path))?;
            let policy = query::trace_check_from_str_with_options(&callsieve_trace_json, true)?;
            let policy_value = serde_json::to_value(policy)?;
            summary_number(&policy_value, "violations")
        } else {
            1
        };
        push_qa(
            &mut results,
            &task.id,
            "strict_trace_policy",
            violations == 0,
            "strict trace policy passed".to_string(),
            format!("strict trace policy violations: {violations}"),
        );
        let observed_collection = trace_value
            .get("metadata")
            .and_then(|metadata| metadata.get("collection"))
            .and_then(serde_json::Value::as_str)
            == Some("observed_session");
        push_qa(
            &mut results,
            &task.id,
            "observed_collection",
            observed_collection,
            "trace metadata collection is observed_session".to_string(),
            "trace metadata collection is not observed_session".to_string(),
        );
        let no_controlled_markers = !trace_has_controlled_replay_marker(&trace_json);
        push_qa(
            &mut results,
            &task.id,
            "controlled_replay_markers",
            no_controlled_markers,
            "trace contains no controlled replay markers".to_string(),
            "trace contains controlled replay markers".to_string(),
        );
        let no_hook_trace_markers = !trace_has_hook_trace_marker(&trace_json);
        push_qa(
            &mut results,
            &task.id,
            "hook_trace_markers",
            no_hook_trace_markers,
            "trace contains no lifecycle hook trace markers".to_string(),
            "trace contains lifecycle hook trace markers".to_string(),
        );
        let trace_token_source = trace_token_accounting_source(&trace_value);
        let transcript_token_accounting = if require_transcript_token_accounting {
            trace_token_source == "transcript_context_tokens"
        } else {
            !trace_token_source.is_empty()
        };
        push_qa(
            &mut results,
            &task.id,
            "trace_token_source",
            transcript_token_accounting,
            "trace token source is auditable".to_string(),
            format!("trace token source is invalid: {trace_token_source}"),
        );

        let countable = preregistered
            && registered_token_source
            && task_text_matches
            && complete_metrics
            && critical_misses == 0
            && violations == 0
            && observed_collection
            && no_controlled_markers
            && no_hook_trace_markers
            && transcript_token_accounting;
        push_qa(
            &mut results,
            &task.id,
            "countable_observed_session",
            countable,
            "session is countable observed proof evidence".to_string(),
            "session is not countable observed proof evidence".to_string(),
        );
    }

    push_qa(
        &mut results,
        "pilot",
        "minimum_planned_tasks",
        manifest.tasks.len() >= minimum_planned_tasks,
        format!(
            "planned tasks {} meet minimum {}",
            manifest.tasks.len(),
            minimum_planned_tasks
        ),
        format!(
            "planned tasks {} are below minimum {}",
            manifest.tasks.len(),
            minimum_planned_tasks
        ),
    );
    push_qa(
        &mut results,
        "pilot",
        "minimum_external_repos",
        external_repos.len() >= minimum_external_repos,
        format!(
            "external repos {} meet minimum {}",
            external_repos.len(),
            minimum_external_repos
        ),
        format!(
            "external repos {} are below minimum {}",
            external_repos.len(),
            minimum_external_repos
        ),
    );
    push_qa(
        &mut results,
        "pilot",
        "minimum_observed_sessions",
        complete_observed_sessions >= manifest.target_sessions,
        format!(
            "observed paired sessions {} meet target {}",
            complete_observed_sessions, manifest.target_sessions
        ),
        format!(
            "observed paired sessions {} are below target {}",
            complete_observed_sessions, manifest.target_sessions
        ),
    );

    let failures = results
        .iter()
        .filter(|result| result.status == "fail")
        .count();
    Ok(PilotQaOutput {
        command: "pilot-qa",
        manifest: manifest_path.display().to_string(),
        status: if failures == 0 { "pass" } else { "fail" }.to_string(),
        target_sessions: manifest.target_sessions,
        observed_sessions: complete_observed_sessions,
        rejected_sessions: manifest.rejected_sessions.len(),
        tasks: manifest.tasks.len(),
        failures,
        results,
    })
}

fn pilot_finalize(
    manifest_path: &Path,
    out: &Path,
    limit: usize,
    snippets_per_file: usize,
    include_snippets: bool,
) -> Result<PilotFinalizeOutput> {
    let qa = pilot_qa(manifest_path)?;
    if qa.status != "pass" {
        anyhow::bail!(
            "pilot QA failed; run `callsieve pilot-qa {}`",
            manifest_path.display()
        );
    }
    let manifest = read_pilot_manifest(manifest_path)?;
    let proof_manifest_path = proof_manifest_path(out);
    let proof_manifest = build_proof_manifest(manifest_path, &manifest)?;
    if let Some(parent) = proof_manifest_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(
        &proof_manifest_path,
        serde_json::to_vec_pretty(&proof_manifest)?,
    )
    .with_context(|| format!("failed to write {}", proof_manifest_path.display()))?;
    let proof_manifest: query::BenchmarkReportManifest = serde_json::from_value(proof_manifest)?;
    let proof = query::proof_report(proof_manifest, limit, snippets_per_file, include_snippets)?;
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, serde_json::to_vec_pretty(&proof)?)
        .with_context(|| format!("failed to write {}", out.display()))?;

    Ok(PilotFinalizeOutput {
        command: "pilot-finalize",
        manifest: manifest_path.display().to_string(),
        proof_manifest: proof_manifest_path.display().to_string(),
        out: out.display().to_string(),
        qa,
        proof,
    })
}

fn read_pilot_manifest(path: &Path) -> Result<PilotHarnessManifest> {
    let json =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&json).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_pilot_manifest(path: &Path, manifest: &PilotHarnessManifest) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, serde_json::to_vec_pretty(manifest)?)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn next_pilot_task_id(manifest: &PilotHarnessManifest) -> String {
    let mut next = manifest.tasks.len() + 1;
    loop {
        let id = format!("task-{next:03}");
        if !manifest.tasks.iter().any(|task| task.id == id) {
            return id;
        }
        next += 1;
    }
}

fn pilot_artifact_root(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn ensure_pilot_trace(task: &PilotHarnessTask, trace_path: &Path) -> Result<()> {
    if trace_path.exists() {
        return Ok(());
    }
    let client = agent_client_from_name(&task.client);
    session_start(
        Path::new(&task.repo),
        &task.task,
        client,
        &task.model,
        trace_path,
        task.expected_files.clone(),
        task.critical_files.clone(),
    )?;
    Ok(())
}

fn agent_client_from_name(name: &str) -> AgentClient {
    match name.to_ascii_lowercase().as_str() {
        "codex" => AgentClient::Codex,
        "claude" => AgentClient::Claude,
        "copilot" => AgentClient::Copilot,
        "opencode" => AgentClient::OpenCode,
        "antigravity" => AgentClient::Antigravity,
        "cursor" => AgentClient::Cursor,
        "vscode" => AgentClient::Vscode,
        "windsurf" => AgentClient::Windsurf,
        "continue" => AgentClient::Continue,
        "zed" => AgentClient::Zed,
        "junie" => AgentClient::Junie,
        "jetbrains" => AgentClient::JetBrains,
        "amp" => AgentClient::Amp,
        "goose" => AgentClient::Goose,
        "warp" => AgentClient::Warp,
        "cline" => AgentClient::Cline,
        "zoo" => AgentClient::Zoo,
        "roo" => AgentClient::Roo,
        _ => AgentClient::Generic,
    }
}

fn pilot_task_status(summary: &serde_json::Value) -> String {
    let baseline_tokens = summary_number(summary, "baseline_tokens");
    let callsieve_tokens = summary_number(summary, "callsieve_tokens");
    let critical_misses = summary_number(summary, "critical_files_still_missed");

    if baseline_tokens > 0 && callsieve_tokens > 0 && critical_misses == 0 {
        "complete".to_string()
    } else if baseline_tokens > 0 && callsieve_tokens == 0 {
        "baseline_recorded".to_string()
    } else if baseline_tokens == 0 && callsieve_tokens > 0 {
        "callsieve_recorded".to_string()
    } else {
        "pending".to_string()
    }
}

fn summary_number(summary: &serde_json::Value, key: &str) -> usize {
    summary
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}

fn threshold_number(thresholds: &serde_json::Value, key: &str) -> usize {
    thresholds
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default()
}

fn threshold_bool(thresholds: &serde_json::Value, key: &str) -> bool {
    thresholds
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn trace_token_accounting_source(trace: &serde_json::Value) -> String {
    trace
        .get("token_accounting")
        .and_then(|accounting| accounting.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn push_qa(
    results: &mut Vec<PilotQaCheck>,
    task_id: &str,
    check: &str,
    pass: bool,
    pass_message: String,
    fail_message: String,
) {
    results.push(PilotQaCheck {
        task_id: task_id.to_string(),
        check: check.to_string(),
        status: if pass { "pass" } else { "fail" }.to_string(),
        message: if pass { pass_message } else { fail_message },
    });
}

fn trace_has_controlled_replay_marker(trace_json: &str) -> bool {
    let lower = trace_json.to_ascii_lowercase();
    [
        "controlled local replay",
        "deterministic local grep/read replay",
        "baseline simulates grepping",
        "callsieve codex-session",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn trace_has_hook_trace_marker(trace_json: &str) -> bool {
    let lower = trace_json.to_ascii_lowercase();
    [
        "_hook_trace",
        "codex_lifecycle_hooks",
        "claude_code_lifecycle_hooks",
        "github_copilot_lifecycle_hooks",
        "antigravity_cli_lifecycle_hooks",
        "cline_lifecycle_hooks",
        "opencode_plugin_hooks",
        "\"hook_event\"",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn proof_manifest_path(out: &Path) -> PathBuf {
    let stem = out
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("proof-report");
    out.with_file_name(format!("{stem}.manifest.json"))
}

fn build_proof_manifest(
    manifest_path: &Path,
    manifest: &PilotHarnessManifest,
) -> Result<serde_json::Value> {
    let root = pilot_artifact_root(manifest_path);
    let suite_root = root.join("suites");
    let mut grouped: BTreeMap<String, Vec<&PilotHarnessTask>> = BTreeMap::new();
    for task in &manifest.tasks {
        if task.status == "rejected" {
            continue;
        }
        grouped.entry(task.repo.clone()).or_default().push(task);
    }
    let token_sources: Vec<String> = manifest
        .tasks
        .iter()
        .map(|task| task.token_accounting_source.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut repos = Vec::new();
    for (repo, tasks) in grouped {
        let label = safe_pilot_label(&repo);
        let suite_path = suite_root.join(format!("{label}.json"));
        if let Some(parent) = suite_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let suite_tasks: Vec<serde_json::Value> = tasks
            .iter()
            .map(|task| {
                serde_json::json!({
                    "id": task.id,
                    "task": task.task,
                    "expected_files": task.expected_files,
                    "critical_files": task.critical_files,
                    "pair_id": task.pair_id.as_deref().unwrap_or(&task.id),
                    "task_category": task.task_category,
                    "difficulty": task.difficulty,
                    "condition": task.condition,
                    "token_accounting_source": task.token_accounting_source
                })
            })
            .collect();
        fs::write(
            &suite_path,
            serde_json::to_vec_pretty(&serde_json::json!({ "tasks": suite_tasks }))?,
        )
        .with_context(|| format!("failed to write {}", suite_path.display()))?;

        let trace_paths: Vec<String> = tasks
            .iter()
            .filter(|task| Path::new(&task.trace_path).is_file())
            .map(|task| task.trace_path.clone())
            .collect();
        let policy_trace_paths: Vec<String> = tasks
            .iter()
            .filter(|task| Path::new(&task.callsieve_trace_path).is_file())
            .map(|task| task.callsieve_trace_path.clone())
            .collect();
        let clients: Vec<String> = tasks
            .iter()
            .map(|task| task.client.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let task_categories: Vec<String> = tasks
            .iter()
            .map(|task| task.task_category.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        repos.push(serde_json::json!({
            "label": label,
            "path": repo,
            "external": tasks.iter().any(|task| task.external),
            "clients": clients,
            "task_categories": task_categories,
            "suite_path": suite_path.display().to_string(),
            "trace_paths": trace_paths,
            "policy_trace_paths": policy_trace_paths
        }));
    }

    Ok(serde_json::json!({
        "protocol": manifest.protocol.evidence_standard,
        "thresholds": manifest.thresholds,
        "audit": {
            "protocol": manifest.protocol,
            "planned_tasks": manifest.tasks.len(),
            "rejected_sessions": manifest.rejected_sessions.len(),
            "token_accounting_sources": token_sources,
            "rejection_audit": manifest.rejected_sessions
        },
        "repos": repos
    }))
}

fn safe_pilot_label(value: &str) -> String {
    let mut label = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            label.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            label.push('-');
            last_dash = true;
        }
    }
    let trimmed = label.trim_matches('-');
    if trimmed.is_empty() {
        "repo".to_string()
    } else {
        trimmed.to_string()
    }
}

fn read_trace_value(path: &Path) -> Result<serde_json::Value> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("failed to read trace: {}", path.display()))?;
    serde_json::from_str(&json)
        .with_context(|| format!("failed to parse trace: {}", path.display()))
}

#[derive(Default)]
struct SessionMetricTotals {
    grep_commands: usize,
    file_reads: usize,
    tokens: usize,
    commands: Vec<String>,
    files_read: Vec<String>,
    context_selected_files: Vec<String>,
}

fn normalize_session_trace(value: &mut serde_json::Value) -> Result<()> {
    let events = value
        .get("events")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut baseline = SessionMetricTotals::default();
    let mut callsieve = SessionMetricTotals::default();
    let mut callsieve_seen = false;

    for event in events {
        let command = event
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if is_callsieve_context_command_local(command) {
            callsieve_seen = true;
        }
        let phase = event
            .get("phase")
            .and_then(serde_json::Value::as_str)
            .unwrap_or({
                if callsieve_seen {
                    "callsieve"
                } else {
                    "baseline"
                }
            });
        let target = if phase == "baseline" {
            &mut baseline
        } else {
            &mut callsieve
        };
        add_event_to_metrics(target, command, &event);
    }

    baseline.files_read.sort();
    baseline.files_read.dedup();
    baseline.context_selected_files.sort();
    baseline.context_selected_files.dedup();
    callsieve.files_read.sort();
    callsieve.files_read.dedup();
    callsieve.context_selected_files.sort();
    callsieve.context_selected_files.dedup();

    let baseline_value = session_metrics_value(&baseline);
    let callsieve_value = session_metrics_value(&callsieve);
    let expected_files = json_string_array(value.get("expected_files"));
    let callsieve_files: std::collections::BTreeSet<&str> = callsieve
        .files_read
        .iter()
        .chain(callsieve.context_selected_files.iter())
        .map(String::as_str)
        .collect();
    let misses: Vec<String> = expected_files
        .into_iter()
        .filter(|file| !callsieve_files.contains(file.as_str()))
        .collect();
    let token_savings = baseline.tokens as isize - callsieve.tokens as isize;
    let token_reduction_percent = if baseline.tokens == 0 {
        0.0
    } else {
        (token_savings as f64 / baseline.tokens as f64) * 100.0
    };
    let token_source = value
        .get("token_accounting")
        .and_then(|accounting| accounting.get("source"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("transcript_context_tokens")
        .to_string();
    let fallback_policy = value
        .get("token_accounting")
        .and_then(|accounting| accounting.get("fallback_policy"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("local_estimator_allowed_only_when_labeled_separately")
        .to_string();

    let object = value
        .as_object_mut()
        .context("session trace root must be a JSON object")?;
    object.insert("baseline".to_string(), baseline_value.clone());
    object.insert("callsieve".to_string(), callsieve_value.clone());
    object.insert(
        "session".to_string(),
        serde_json::json!({
            "baseline": baseline_value,
            "callsieve": callsieve_value
        }),
    );
    object.insert("misses".to_string(), serde_json::json!(misses));
    object.insert(
        "token_accounting".to_string(),
        serde_json::json!({
            "source": token_source,
            "fallback_policy": fallback_policy,
            "baseline_tokens": baseline.tokens,
            "callsieve_tokens": callsieve.tokens,
            "token_savings": token_savings,
            "token_reduction_percent": token_reduction_percent
        }),
    );
    if let Some(metadata) = object
        .get_mut("metadata")
        .and_then(serde_json::Value::as_object_mut)
    {
        metadata.insert(
            "updated_at".to_string(),
            serde_json::Value::from(now_unix_seconds()),
        );
    }
    Ok(())
}

fn add_event_to_metrics(
    metrics: &mut SessionMetricTotals,
    command: &str,
    event: &serde_json::Value,
) {
    let classification = event
        .get("classification")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| classify_session_command(command));
    let files_read = json_string_array(event.get("files_read"));
    let context_selected_files = json_string_array(event.get("context_selected_files"));
    if classification == "grep" || is_grep_command_local(command) {
        metrics.grep_commands += 1;
    }
    if !files_read.is_empty() {
        metrics.file_reads += files_read.len();
        metrics.files_read.extend(files_read);
    } else if classification == "file_read" || is_file_read_command_local(command) {
        metrics.file_reads += 1;
    }
    if !context_selected_files.is_empty() {
        metrics
            .context_selected_files
            .extend(context_selected_files);
    }
    metrics.tokens += event
        .get("tokens")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();
    if !command.is_empty() {
        metrics.commands.push(command.to_string());
    }
}

fn session_metrics_value(metrics: &SessionMetricTotals) -> serde_json::Value {
    serde_json::json!({
        "grep_commands": metrics.grep_commands,
        "file_reads": metrics.file_reads,
        "tokens": metrics.tokens,
        "commands": metrics.commands.clone(),
        "files_read": metrics.files_read.clone(),
        "context_selected_files": metrics.context_selected_files.clone()
    })
}

fn empty_session_metrics() -> serde_json::Value {
    serde_json::json!({
        "grep_commands": 0,
        "file_reads": 0,
        "tokens": 0,
        "commands": [],
        "files_read": [],
        "context_selected_files": []
    })
}

fn first_required_context_command(root: &Path, task: &str) -> String {
    format!("callsieve agent-context {} {:?}", root.display(), task)
}

fn session_phase_name(phase: SessionPhase) -> &'static str {
    match phase {
        SessionPhase::Baseline => "baseline",
        SessionPhase::Callsieve => "callsieve",
    }
}

fn infer_session_phase(value: &serde_json::Value, command: &str) -> &'static str {
    if is_callsieve_context_command_local(command) {
        return "callsieve";
    }
    let callsieve_seen = value
        .get("events")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|event| {
            event
                .get("phase")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|phase| phase == "callsieve")
                || event
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(is_callsieve_context_command_local)
        });
    if callsieve_seen {
        "callsieve"
    } else {
        "baseline"
    }
}

fn classify_session_command(command: &str) -> &'static str {
    if is_callsieve_context_command_local(command) {
        "callsieve_context"
    } else if is_broad_search_command_local(command) {
        "grep"
    } else if is_file_read_command_local(command) {
        "file_read"
    } else {
        "other"
    }
}

fn json_string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect()
}

fn is_grep_command_local(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    matches!(first, "rg" | "grep" | "ripgrep")
        || lower.contains(" rg ")
        || lower.contains(" grep ")
        || lower.contains("ripgrep")
}

fn is_broad_search_command_local(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    is_grep_command_local(command)
        || lower.starts_with("git grep")
        || lower.contains(" git grep ")
        || matches!(
            first,
            "find" | "fd" | "glob" | "grep_search" | "find_by_name" | "codebase_search"
        )
        || lower.contains(" grep_search")
        || lower.contains(" find_by_name")
        || lower.contains(" codebase_search")
        || lower.contains(" select-string ")
        || lower.starts_with("select-string ")
        || (lower.contains("get-childitem") && lower.contains("-recurse"))
        || (lower.starts_with("dir ") && lower.contains("/s"))
        || (lower.starts_with("ls ") && lower.contains("-r"))
}

fn is_file_read_command_local(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    matches!(
        first,
        "cat"
            | "less"
            | "more"
            | "head"
            | "tail"
            | "sed"
            | "nl"
            | "bat"
            | "type"
            | "get-content"
            | "read"
            | "read_file"
            | "view_file"
    ) || lower.contains(" get-content ")
        || lower.starts_with("read_file")
        || lower.starts_with("view_file")
        || lower.contains(" read_file")
        || lower.contains(" view_file")
}

fn is_callsieve_context_command_local(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("callsieve context")
        || lower.contains("callsieve agent-context")
        || lower.contains("callsieve codex-session")
        || lower.contains("callsieve session-start")
        || lower.contains("callsieve begin")
        || lower.contains("callsieve_context")
        || lower.contains("callsieve guard")
        || lower.contains("callsieve grep")
}

fn setup_agent(client: AgentClient, root: &Path, force: bool) -> Result<SetupAgentOutput> {
    let first_required_command = format!("callsieve agent-context {} \"<task>\"", root.display());
    let files = agent_files(client, root);
    let mut written = Vec::new();
    let warnings = agent_client_warnings_for_root(client, root);

    for (path, content) in files {
        if path.exists() && !force {
            anyhow::bail!(
                "refusing to overwrite {}; pass --force to replace it",
                path.display()
            );
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))?;
        written.push(repo_relative_display(root, &path));
    }
    if matches!(client, AgentClient::Zoo | AgentClient::Roo)
        && (force || root.join(".roomodes").is_file())
    {
        write_project_file(
            root,
            &root.join(".roomodes"),
            &zoo_roomodes_json(),
            true,
            &mut written,
        )?;
    }
    written.sort();
    written.dedup();

    Ok(SetupAgentOutput {
        command: "setup-agent",
        client: agent_client_name(client).to_string(),
        root: root_label(root),
        files: written,
        first_required_command,
        policy: "Call callsieve_context before broad grep, rg, repository search, or repeated file reads. Read read_first, review context.sel.next, and use instruction.x.o/n/next plus focus, related, and tests before grep.",
        warnings,
    })
}

#[derive(Debug, Serialize)]
struct SetupAutoDetection {
    client: String,
    evidence: String,
}

#[derive(Debug, Serialize)]
struct SetupAutoOutput {
    command: &'static str,
    root: String,
    detected: Vec<SetupAutoDetection>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    configured: Vec<SetupAgentOutput>,
    dry_run: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

/// Local-only signals that an agent is installed: a binary on PATH, a config
/// directory under the home directory, a macOS app bundle, or a VS Code
/// extension prefix. No network calls.
struct AgentDetectionSignals {
    client: AgentClient,
    binaries: &'static [&'static str],
    home_dirs: &'static [&'static str],
    app_bundles: &'static [&'static str],
    extension_prefixes: &'static [&'static str],
}

fn agent_detection_table() -> Vec<AgentDetectionSignals> {
    // (client, binaries, home-relative dirs, app bundles, vscode extension prefixes)
    let table = vec![
        (
            AgentClient::Claude,
            &["claude"][..],
            &[".claude"][..],
            &[][..],
            &[][..],
        ),
        (AgentClient::Codex, &["codex"], &[".codex"], &[], &[]),
        (
            AgentClient::Copilot,
            &["copilot", "github-copilot-cli"],
            &[".config/github-copilot"],
            &[],
            &["github.copilot"],
        ),
        (
            AgentClient::OpenCode,
            &["opencode"],
            &[".opencode", ".config/opencode"],
            &[],
            &[],
        ),
        (
            AgentClient::Antigravity,
            &["antigravity"],
            &[],
            &["Antigravity.app"],
            &[],
        ),
        (
            AgentClient::Cursor,
            &["cursor"],
            &[".cursor", ".config/Cursor"],
            &["Cursor.app"],
            &[],
        ),
        (
            AgentClient::Vscode,
            &["code"],
            &[".vscode", ".config/Code"],
            &["Visual Studio Code.app"],
            &[],
        ),
        (
            AgentClient::Windsurf,
            &["windsurf"],
            &[".windsurf", ".codeium/windsurf", ".config/Windsurf"],
            &["Windsurf.app"],
            &[],
        ),
        (
            AgentClient::Continue,
            &[],
            &[".continue"],
            &[],
            &["continue.continue"],
        ),
        (
            AgentClient::Zed,
            &["zed"],
            &[".config/zed"],
            &["Zed.app"],
            &[],
        ),
        (
            AgentClient::JetBrains,
            &[],
            &[".config/JetBrains", "Library/Application Support/JetBrains"],
            &[],
            &[],
        ),
        (
            AgentClient::Amp,
            &["amp"],
            &[".config/amp"],
            &[],
            &["sourcegraph.amp"],
        ),
        (AgentClient::Goose, &["goose"], &[".config/goose"], &[], &[]),
        (AgentClient::Warp, &[], &[".warp"], &["Warp.app"], &[]),
        (
            AgentClient::Cline,
            &[],
            &[],
            &[],
            &["saoudrizwan.claude-dev"],
        ),
        (AgentClient::Roo, &[], &[], &[], &["rooveterinaryinc"]),
    ];
    table
        .into_iter()
        .map(
            |(client, binaries, home_dirs, app_bundles, extension_prefixes)| {
                AgentDetectionSignals {
                    client,
                    binaries,
                    home_dirs,
                    app_bundles,
                    extension_prefixes,
                }
            },
        )
        .collect()
}

fn detect_agent_clients(
    home: &Path,
    path_dirs: &[PathBuf],
    applications: &Path,
) -> Vec<(AgentClient, String)> {
    let vscode_extensions: Vec<String> = fs::read_dir(home.join(".vscode/extensions"))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().to_str().map(str::to_ascii_lowercase))
                .collect()
        })
        .unwrap_or_default();

    let mut detected = Vec::new();
    for AgentDetectionSignals {
        client,
        binaries,
        home_dirs,
        app_bundles,
        extension_prefixes,
    } in agent_detection_table()
    {
        let binary_hit = binaries.iter().find_map(|binary| {
            path_dirs
                .iter()
                .map(|dir| dir.join(binary))
                .find(|candidate| candidate.is_file())
                .map(|candidate| format!("binary: {}", candidate.display()))
        });
        let dir_hit = || {
            home_dirs
                .iter()
                .map(|dir| home.join(dir))
                .find(|candidate| candidate.is_dir())
                .map(|candidate| format!("directory: {}", candidate.display()))
        };
        let app_hit = || {
            app_bundles
                .iter()
                .map(|bundle| applications.join(bundle))
                .find(|candidate| candidate.exists())
                .map(|candidate| format!("application: {}", candidate.display()))
        };
        let extension_hit = || {
            extension_prefixes
                .iter()
                .find(|prefix| {
                    vscode_extensions
                        .iter()
                        .any(|extension| extension.starts_with(*prefix))
                })
                .map(|prefix| format!("vscode extension: {prefix}"))
        };
        if let Some(evidence) = binary_hit
            .or_else(dir_hit)
            .or_else(app_hit)
            .or_else(extension_hit)
        {
            detected.push((client, evidence));
        }
    }
    detected
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ReceiptFormat {
    Json,
    Markdown,
}

/// Per-session retrieval receipt: locally observed counts only — no
/// estimated-savings claims — plus a content hash so tampering with a stored
/// receipt is evident. The hash is FNV-1a, deliberately labeled
/// tamper-evident rather than cryptographic.
#[derive(Debug, Serialize)]
struct SessionReceipt {
    command: &'static str,
    root: String,
    session_id: String,
    client: String,
    callsieve_version: &'static str,
    index_fingerprint: String,
    context_packets: usize,
    context_tokens: usize,
    file_reads: usize,
    grep_commands: usize,
    policy_violations: usize,
    edit_impacts: usize,
    first_event_at: u64,
    last_event_at: u64,
    trace_path: String,
    content_hash: String,
}

fn hook_trace_files(root: &Path) -> Vec<PathBuf> {
    let mut traces = Vec::new();
    let base = callsieve_dir(root);
    let Ok(entries) = fs::read_dir(&base) else {
        return traces;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let dir = entry.path();
        if !dir.is_dir()
            || !dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-hooks"))
        {
            continue;
        }
        if let Ok(files) = fs::read_dir(&dir) {
            for file in files.filter_map(|file| file.ok()) {
                let path = file.path();
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".trace.json"))
                {
                    traces.push(path);
                }
            }
        }
    }
    traces.sort();
    traces
}

fn current_index_fingerprint(root: &Path) -> String {
    store::json_store::load_index(root)
        .map(|index| indexer::index_fingerprint(&index))
        .unwrap_or_default()
}

fn receipt_from_trace(root: &Path, trace_path: &Path) -> Result<SessionReceipt> {
    receipt_from_trace_with_fingerprint(root, trace_path, &current_index_fingerprint(root))
}

fn receipt_from_trace_with_fingerprint(
    root: &Path,
    trace_path: &Path,
    fingerprint: &str,
) -> Result<SessionReceipt> {
    let raw = fs::read_to_string(trace_path)
        .with_context(|| format!("failed to read {}", trace_path.display()))?;
    let trace: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", trace_path.display()))?;
    let metadata = trace.get("metadata").cloned().unwrap_or_default();
    let session_id = metadata
        .get("session_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let client = metadata
        .get("client")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();

    let empty = Vec::new();
    let events = trace
        .get("events")
        .and_then(serde_json::Value::as_array)
        .unwrap_or(&empty);
    let classification = |event: &serde_json::Value| -> String {
        event
            .get("classification")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut receipt = SessionReceipt {
        command: "receipt",
        root: root_label(root),
        session_id,
        client,
        callsieve_version: env!("CARGO_PKG_VERSION"),
        index_fingerprint: fingerprint.to_string(),
        context_packets: 0,
        context_tokens: 0,
        file_reads: 0,
        grep_commands: 0,
        policy_violations: 0,
        edit_impacts: 0,
        first_event_at: u64::MAX,
        last_event_at: 0,
        trace_path: repo_relative_display(root, trace_path),
        content_hash: String::new(),
    };
    for event in events {
        match classification(event).as_str() {
            "callsieve_context" => {
                receipt.context_packets += 1;
                receipt.context_tokens += event
                    .get("tokens")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default() as usize;
            }
            "file_read" => receipt.file_reads += 1,
            "grep" => receipt.grep_commands += 1,
            "edit_impact" => receipt.edit_impacts += 1,
            _ => {}
        }
        if event
            .get("policy_violation")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            receipt.policy_violations += 1;
        }
        if let Some(timestamp) = event.get("timestamp").and_then(serde_json::Value::as_u64) {
            receipt.first_event_at = receipt.first_event_at.min(timestamp);
            receipt.last_event_at = receipt.last_event_at.max(timestamp);
        }
    }
    if receipt.first_event_at == u64::MAX {
        receipt.first_event_at = 0;
    }
    receipt.content_hash =
        indexer::stable_content_hash(serde_json::to_string(&receipt)?.as_bytes());
    Ok(receipt)
}

fn session_receipt(root: &Path, session: Option<&str>) -> Result<SessionReceipt> {
    let traces = hook_trace_files(root);
    let trace_path = match session {
        Some(session) => {
            let sanitized = format!("{}.trace.json", safe_pilot_label(session));
            traces
                .into_iter()
                .find(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name == format!("{session}.trace.json") || name == sanitized
                        })
                })
                .with_context(|| format!("no session trace found for {session}"))?
        }
        None => traces
            .into_iter()
            .max_by_key(|path| {
                fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
            })
            .context("no session traces recorded under .callsieve/*-hooks/")?,
    };
    receipt_from_trace(root, &trace_path)
}

fn receipt_markdown(receipt: &SessionReceipt) -> String {
    format!(
        "# CallSieve retrieval receipt\n\n- Session: `{}` ({})\n- Repo: {} (index `{}`)\n- Context packets: {} (~{} tokens)\n- File reads: {}  |  Broad searches: {}  |  Policy violations: {}\n- Edit-impact notes: {}\n- Window: {} -> {}\n- callsieve v{}  |  content hash (tamper-evident): `{}`\n",
        receipt.session_id,
        receipt.client,
        receipt.root,
        receipt.index_fingerprint,
        receipt.context_packets,
        receipt.context_tokens,
        receipt.file_reads,
        receipt.grep_commands,
        receipt.policy_violations,
        receipt.edit_impacts,
        receipt.first_event_at,
        receipt.last_event_at,
        receipt.callsieve_version,
        receipt.content_hash,
    )
}

#[derive(Debug, Serialize)]
struct SessionReceiptsRollup {
    command: &'static str,
    root: String,
    sessions: usize,
    context_packets: usize,
    context_tokens: usize,
    file_reads: usize,
    grep_commands: usize,
    policy_violations: usize,
    edit_impacts: usize,
    content_hash: String,
}

fn session_receipts_rollup(root: &Path) -> Result<SessionReceiptsRollup> {
    let mut rollup = SessionReceiptsRollup {
        command: "receipts",
        root: root_label(root),
        sessions: 0,
        context_packets: 0,
        context_tokens: 0,
        file_reads: 0,
        grep_commands: 0,
        policy_violations: 0,
        edit_impacts: 0,
        content_hash: String::new(),
    };
    let fingerprint = current_index_fingerprint(root);
    for trace_path in hook_trace_files(root) {
        let Ok(receipt) = receipt_from_trace_with_fingerprint(root, &trace_path, &fingerprint)
        else {
            continue;
        };
        rollup.sessions += 1;
        rollup.context_packets += receipt.context_packets;
        rollup.context_tokens += receipt.context_tokens;
        rollup.file_reads += receipt.file_reads;
        rollup.grep_commands += receipt.grep_commands;
        rollup.policy_violations += receipt.policy_violations;
        rollup.edit_impacts += receipt.edit_impacts;
    }
    rollup.content_hash = indexer::stable_content_hash(serde_json::to_string(&rollup)?.as_bytes());
    Ok(rollup)
}

#[derive(Debug, Serialize)]
struct IndexExportOutput {
    command: &'static str,
    root: String,
    out: String,
    files: usize,
    symbols: usize,
    references: usize,
    schema_version: u32,
    fingerprint: String,
    bytes: u64,
}

fn index_export(root: &Path, out: &Path) -> Result<IndexExportOutput> {
    let index = store::json_store::load_index(root)?;
    if index.schema_version != indexer::SCHEMA_VERSION {
        anyhow::bail!(
            "index schema {} does not match current schema {}; run `callsieve index {}` before exporting",
            index.schema_version,
            indexer::SCHEMA_VERSION,
            root.display()
        );
    }
    let data = serde_json::to_vec(&index)?;
    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, &data).with_context(|| format!("failed to write {}", out.display()))?;
    Ok(IndexExportOutput {
        command: "index-export",
        root: root_label(root),
        out: out.display().to_string(),
        files: index.files.len(),
        symbols: index.symbols.len(),
        references: index.references.len(),
        schema_version: index.schema_version,
        fingerprint: indexer::index_fingerprint(&index),
        bytes: data.len() as u64,
    })
}

#[derive(Debug, Serialize)]
struct IndexImportOutput {
    command: &'static str,
    root: String,
    from: String,
    files_total: usize,
    files_matched: usize,
    files_mismatched: usize,
    files_missing: usize,
    fingerprint: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
}

/// Verification is by content hash, not mtime: an exported index carries the
/// builder's mtimes, which never match another machine's checkout. Matched
/// files get their mtime/size rewritten from local disk so the freshness
/// check passes; mismatched files keep the stale stats on purpose so the
/// next rebuild picks them up.
fn index_import(root: &Path, from: &Path, allow_partial: bool) -> Result<IndexImportOutput> {
    let data = fs::read(from).with_context(|| format!("failed to read {}", from.display()))?;
    let mut index: store::CodeIndex = serde_json::from_slice(&data)
        .with_context(|| format!("failed to parse {}", from.display()))?;
    store::normalize_after_load(&mut index);
    if index.schema_version != indexer::SCHEMA_VERSION {
        anyhow::bail!(
            "exported index schema {} does not match this binary's schema {}; re-export with a matching callsieve version",
            index.schema_version,
            indexer::SCHEMA_VERSION
        );
    }

    let root_canonical = root
        .canonicalize()
        .with_context(|| format!("failed to resolve repo root {}", root.display()))?;
    let mut matched = 0usize;
    let mut mismatched = 0usize;
    let mut missing = 0usize;
    for file in &mut index.files {
        let local = root_canonical.join(&file.path);
        match fs::read(&local) {
            Ok(bytes) => {
                if indexer::stable_content_hash(&bytes) == file.content_hash {
                    if let Ok(metadata) = fs::metadata(&local) {
                        file.mtime = indexer::metadata_mtime(&metadata);
                        file.size_bytes = metadata.len();
                    }
                    matched += 1;
                } else {
                    mismatched += 1;
                }
            }
            Err(_) => missing += 1,
        }
    }

    let total = index.files.len();
    let drift = mismatched + missing;
    if drift > 0 && !allow_partial {
        anyhow::bail!(
            "{drift} of {total} indexed files differ from this checkout ({mismatched} changed, {missing} missing); pass --allow-partial to import anyway or run `callsieve index {}`",
            root.display()
        );
    }
    if allow_partial && drift * 10 > total {
        anyhow::bail!(
            "{drift} of {total} indexed files differ from this checkout, which exceeds the 10% --allow-partial budget; run `callsieve index {}` instead",
            root.display()
        );
    }

    let mut warnings = Vec::new();
    if drift > 0 {
        warnings.push(format!(
            "{drift} file(s) differ from the export and stay stale until the next rebuild"
        ));
    }
    warnings
        .push("files added after the export are not indexed until the next rebuild".to_string());

    index.metadata.indexed_at = now_unix_seconds();
    index.metadata.index_generation = 0;
    index.metadata.watch_status = "unwatched".to_string();
    index.metadata.watcher_mode = "none".to_string();
    index.metadata.last_error = None;
    store::json_store::save_index(root, &index)?;

    Ok(IndexImportOutput {
        command: "index-import",
        root: root_label(root),
        from: from.display().to_string(),
        files_total: total,
        files_matched: matched,
        files_mismatched: mismatched,
        files_missing: missing,
        fingerprint: indexer::index_fingerprint(&index),
        warnings,
    })
}

fn setup_auto(root: &Path, force: bool, dry_run: bool) -> Result<SetupAutoOutput> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let detected = detect_agent_clients(&home, &path_dirs, Path::new("/Applications"));
    setup_auto_with_detected(root, force, dry_run, detected)
}

/// Lifecycle hook files for clients that support them, in addition to the
/// rules/config files from `setup_agent`. Hooks are the strongest
/// context-first integration, so the zero-decision path installs them
/// (non-strict) wherever the client allows.
fn setup_auto_hook_files(root: &Path, client: AgentClient, force: bool) -> Result<Vec<String>> {
    match client {
        AgentClient::Codex => {
            write_codex_hooks_files(root, false, force, 6, 1).map(|(_, _, files)| files)
        }
        AgentClient::Claude => {
            write_claude_hooks_files(root, false, force, 6, 1).map(|(_, _, files)| files)
        }
        _ => match hook_client_for_agent(client) {
            Some(hook_client) => write_client_hooks_files(root, hook_client, false, force, 6, 1)
                .map(|(_, _, files)| files),
            None => Ok(Vec::new()),
        },
    }
}

fn setup_auto_with_detected(
    root: &Path,
    force: bool,
    dry_run: bool,
    detected: Vec<(AgentClient, String)>,
) -> Result<SetupAutoOutput> {
    let mut warnings = Vec::new();
    if detected.is_empty() {
        warnings.push(
            "no supported agents detected; run `callsieve setup-agent <client> <repo>` explicitly"
                .to_string(),
        );
    }
    let mut configured = Vec::new();
    if !dry_run {
        for (client, _) in &detected {
            match setup_agent(*client, root, force) {
                Ok(mut output) => {
                    match setup_auto_hook_files(root, *client, force) {
                        Ok(hook_files) => {
                            output.files.extend(hook_files);
                            output.files.sort();
                            output.files.dedup();
                        }
                        Err(error) => warnings.push(format!(
                            "hook setup failed for {}: {error}",
                            agent_client_name(*client)
                        )),
                    }
                    configured.push(output);
                }
                Err(error) => warnings.push(format!(
                    "setup failed for {}: {error}",
                    agent_client_name(*client)
                )),
            }
        }
    }

    Ok(SetupAutoOutput {
        command: "setup-auto",
        root: root_label(root),
        detected: detected
            .into_iter()
            .map(|(client, evidence)| SetupAutoDetection {
                client: agent_client_name(client).to_string(),
                evidence,
            })
            .collect(),
        configured,
        dry_run,
        warnings,
    })
}

fn codex_bootstrap(root: &Path, model: &str, force: bool) -> Result<CodexBootstrapOutput> {
    let setup = setup_agent(AgentClient::Codex, root, force)?;
    let shim = install_shim(root, force, false)?;
    let mut files = setup.files;
    files.extend(shim.files);

    let first_required_command = format!("callsieve agent-context {} \"<task>\"", root.display());
    let launchers = write_codex_launchers(root, &first_required_command, force)?;
    files.extend(launchers.clone());
    let hooks = codex_hooks_install(root, true, force, 6, 1, false)?;
    files.extend(hooks.files);
    files.sort();
    files.dedup();

    Ok(CodexBootstrapOutput {
        command: "codex-bootstrap",
        root: root_label(root),
        model: model.to_string(),
        files,
        first_required_command,
        launcher: launchers,
        policy: "project-local bootstrap only; no global PATH, shell profile, or user config mutation",
    })
}

fn codex_hooks_install(
    root: &Path,
    strict: bool,
    force: bool,
    limit: usize,
    snippets_per_file: usize,
    lsp: bool,
) -> Result<CodexHooksInstallOutput> {
    let index = build_index_output(root, lsp)?;
    let (hooks_file, trace_dir, files) =
        write_codex_hooks_files(root, strict, force, limit, snippets_per_file)?;

    Ok(CodexHooksInstallOutput {
        command: "codex-hooks install",
        status: "pass".to_string(),
        root: root_label(root),
        profile: CODEX_HOOK_PROFILE,
        strict,
        hooks_file,
        trace_dir,
        files,
        index,
        first_required_command: format!("callsieve agent-context {} \"<task>\"", root.display()),
        trust_instruction: "Review and trust project hooks in Codex with /hooks.",
        policy: "Codex lifecycle hooks inject CallSieve context plus local expansion commands and block broad search before context.",
    })
}

fn write_codex_hooks_files(
    root: &Path,
    strict: bool,
    force: bool,
    limit: usize,
    snippets_per_file: usize,
) -> Result<(String, String, Vec<String>)> {
    let mut files = Vec::new();
    let hooks_path = codex_hooks_path(root);
    let config = codex_hooks_json(root, strict, limit, snippets_per_file);
    let config_text = serde_json::to_string_pretty(&config)?;
    write_project_file(root, &hooks_path, &config_text, force, &mut files)?;
    let trace_dir = codex_hook_dir(root);
    fs::create_dir_all(&trace_dir)
        .with_context(|| format!("failed to create {}", trace_dir.display()))?;
    files.push(repo_relative_display(root, &trace_dir));
    files.sort();
    files.dedup();
    Ok((
        repo_relative_display(root, &hooks_path),
        repo_relative_display(root, &trace_dir),
        files,
    ))
}

fn codex_hooks_doctor(root: &Path, strict: bool) -> CodexHooksDoctorOutput {
    codex_hooks_doctor_with_options(root, strict, CodexHooksDoctorOptions::default())
}

fn codex_hooks_doctor_with_options(
    root: &Path,
    strict: bool,
    options: CodexHooksDoctorOptions,
) -> CodexHooksDoctorOutput {
    let hooks_path = codex_hooks_path(root);
    let trace_dir = codex_hook_dir(root);
    let content = fs::read_to_string(&hooks_path).ok();
    let parsed = content
        .as_deref()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok());
    let mut checks = Vec::new();
    checks.push(enforce_check(
        "codex_hooks_file",
        hooks_path.is_file(),
        if hooks_path.is_file() {
            "Codex hooks file exists"
        } else {
            "Codex hooks file is missing"
        },
    ));
    checks.push(enforce_check(
        "codex_hooks_json",
        parsed.is_some(),
        if parsed.is_some() {
            "Codex hooks file is valid JSON"
        } else {
            "Codex hooks file is missing or invalid JSON"
        },
    ));

    for event in CODEX_HOOK_EVENTS {
        let installed = parsed
            .as_ref()
            .and_then(|value| value.get("hooks"))
            .and_then(|hooks| hooks.get(event))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|entries| !entries.is_empty());
        checks.push(enforce_check(
            format!("codex_hook_event:{event}"),
            installed,
            if installed {
                format!("{event} hook is installed")
            } else {
                format!("{event} hook is missing")
            },
        ));
    }

    let command_text = content.as_deref().unwrap_or_default();
    checks.push(enforce_check(
        "codex_hook_commands",
        CODEX_HOOK_COMMAND_NAMES
            .iter()
            .all(|command| command_text.contains(command)),
        "Codex hooks point at CallSieve hook handlers",
    ));
    checks.push(enforce_check(
        "codex_hook_command_windows",
        command_text.contains("commandWindows"),
        "Codex hooks include Windows command strings",
    ));
    checks.push(enforce_check(
        "codex_hook_strict",
        !strict || command_text.contains("--strict"),
        if strict {
            "Codex hooks are installed in strict mode"
        } else {
            "strict Codex hooks are optional"
        },
    ));
    checks.push(check_with_status(
        "codex_hook_trace_dir",
        if trace_dir.is_dir() { "pass" } else { "warn" },
        if trace_dir.is_dir() {
            "Codex hook trace directory exists"
        } else {
            "Codex hook trace directory will be created on first hook run"
        },
    ));
    let hooks_hash = hook_hash(command_text);
    let trust = codex_hook_trust_review(root, &hooks_hash);
    checks.push(check_with_status(
        "codex_hook_trust_ack",
        if trust.status == "reviewed" {
            "pass"
        } else {
            "warn"
        },
        trust.message.clone(),
    ));

    let mut fixes = Vec::new();
    if options.fix {
        match archive_stale_codex_hook_files(root) {
            Ok(archived) => {
                if archived.is_empty() {
                    fixes.push("no stale Codex hook state or trace files found".to_string());
                } else {
                    fixes.extend(archived);
                }
            }
            Err(error) => checks.push(enforce_check(
                "codex_hook_fix_stale_state",
                false,
                format!("failed to archive stale Codex hook state: {error}"),
            )),
        }
    }

    if options.smoke {
        checks.extend(codex_hooks_smoke_checks(root, strict));
    }

    CodexHooksDoctorOutput {
        command: "codex-hooks doctor",
        status: status_from_checks(&checks),
        root: root_label(root),
        profile: CODEX_HOOK_PROFILE,
        strict,
        hooks_file: repo_relative_display(root, &hooks_path),
        trace_dir: repo_relative_display(root, &trace_dir),
        checks,
        fixes,
        trust,
        trust_instruction: "Review and trust project hooks in Codex with /hooks.",
    }
}

#[derive(Debug, Clone, Copy)]
enum HookSmokeExpectation {
    UserPromptSubmit,
    CodexNoop {
        event: &'static str,
    },
    CodexPermissionDecision {
        event: &'static str,
        expected: &'static str,
    },
    EmptyStdout,
}

fn codex_hooks_smoke_checks(root: &Path, strict: bool) -> Vec<EnforceCheck> {
    let session_prefix = format!("callsieve-smoke-{}", now_unix_seconds());
    let cases = vec![
        (
            "codex_hook_smoke:user_prompt_submit",
            "user-prompt-submit",
            vec!["--limit", "1", "--snippets-per-file", "0"],
            serde_json::json!({
                "session_id": format!("{session_prefix}-prompt"),
                "turn_id": "turn-1",
                "prompt": "fix hook doctor smoke"
            })
            .to_string(),
            HookSmokeExpectation::UserPromptSubmit,
        ),
        (
            "codex_hook_smoke:pre_tool_use",
            "pre-tool-use",
            Vec::new(),
            serde_json::json!({
                "session_id": format!("{session_prefix}-pre"),
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": { "command": "rg createSession" }
            })
            .to_string(),
            HookSmokeExpectation::CodexPermissionDecision {
                event: "PreToolUse",
                expected: "deny",
            },
        ),
        (
            "codex_hook_smoke:pre_tool_use_allow",
            "pre-tool-use",
            Vec::new(),
            serde_json::json!({
                "session_id": format!("{session_prefix}-pre-allow"),
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": { "command": "git status --short" }
            })
            .to_string(),
            HookSmokeExpectation::CodexNoop {
                event: "PreToolUse",
            },
        ),
        (
            "codex_hook_smoke:permission_request",
            "permission-request",
            Vec::new(),
            serde_json::json!({
                "session_id": format!("{session_prefix}-permission"),
                "hook_event_name": "PermissionRequest",
                "tool_name": "Bash",
                "tool_input": { "command": "rg createSession" }
            })
            .to_string(),
            HookSmokeExpectation::CodexPermissionDecision {
                event: "PermissionRequest",
                expected: "deny",
            },
        ),
        (
            "codex_hook_smoke:disabled_post_tool_use",
            "post-tool-use",
            Vec::new(),
            serde_json::json!({
                "session_id": format!("{session_prefix}-post"),
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_input": { "command": "sed -n '1,20p' src/cli.rs" }
            })
            .to_string(),
            HookSmokeExpectation::EmptyStdout,
        ),
        (
            "codex_hook_smoke:disabled_stop",
            "stop",
            Vec::new(),
            serde_json::json!({
                "session_id": format!("{session_prefix}-stop"),
                "hook_event_name": "Stop",
                "stop_hook_active": false
            })
            .to_string(),
            HookSmokeExpectation::EmptyStdout,
        ),
    ];

    let checks = cases
        .into_iter()
        .map(|(check, hook_command, extra_args, input, expectation)| {
            run_codex_hook_smoke_case(
                root,
                strict,
                check,
                hook_command,
                &extra_args,
                &input,
                expectation,
            )
        })
        .collect::<Vec<_>>();
    cleanup_codex_smoke_files(root, &session_prefix);
    checks
}

fn run_codex_hook_smoke_case(
    root: &Path,
    strict: bool,
    check: &str,
    hook_command: &str,
    extra_args: &[&str],
    input: &str,
    expectation: HookSmokeExpectation,
) -> EnforceCheck {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from(callsieve_executable_display()));
    let mut child = match ProcessCommand::new(exe)
        .arg("codex-hook")
        .arg(hook_command)
        .arg(root)
        .args(strict.then_some("--strict"))
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return enforce_check(
                check,
                false,
                format!("failed to spawn hook smoke case: {error}"),
            );
        }
    };
    if let Some(stdin) = child.stdin.as_mut()
        && let Err(error) = stdin.write_all(input.as_bytes())
    {
        return enforce_check(
            check,
            false,
            format!("failed to write hook smoke input: {error}"),
        );
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            return enforce_check(
                check,
                false,
                format!("failed to collect hook smoke output: {error}"),
            );
        }
    };
    if !output.status.success() {
        return enforce_check(
            check,
            false,
            format!(
                "hook smoke case exited nonzero: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let validation = validate_hook_smoke_output(&stdout, expectation);
    let passed = validation.is_ok();

    enforce_check(
        check,
        passed,
        if passed {
            "hook smoke case passed".to_string()
        } else {
            format!(
                "{}: {stdout}",
                validation.expect_err("failed validation should have an error")
            )
        },
    )
}

fn validate_hook_smoke_output(
    stdout: &str,
    expectation: HookSmokeExpectation,
) -> std::result::Result<(), String> {
    match expectation {
        HookSmokeExpectation::UserPromptSubmit => {
            let value = parse_hook_smoke_json(stdout, "Codex UserPromptSubmit")?;
            let hook = value.get("hookSpecificOutput").ok_or_else(|| {
                "missing hookSpecificOutput in Codex UserPromptSubmit response".to_string()
            })?;
            let event = hook
                .get("hookEventName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if event != "UserPromptSubmit" {
                return Err(
                    "missing hookSpecificOutput.hookEventName UserPromptSubmit in Codex UserPromptSubmit response"
                        .to_string(),
                );
            }
            if let Some(additional_context) = hook.get("additionalContext")
                && !additional_context.is_string()
            {
                return Err(
                    "invalid hookSpecificOutput.additionalContext in Codex UserPromptSubmit response"
                        .to_string(),
                );
            }
            Ok(())
        }
        HookSmokeExpectation::CodexNoop { event } => {
            validate_codex_noop_smoke_output(stdout, event)
        }
        HookSmokeExpectation::CodexPermissionDecision { event, expected } => {
            validate_codex_permission_smoke_output(stdout, event, expected)
        }
        HookSmokeExpectation::EmptyStdout => stdout
            .trim()
            .is_empty()
            .then_some(())
            .ok_or_else(|| "expected empty stdout from disabled Codex hook".to_string()),
    }
}

fn validate_codex_noop_smoke_output(stdout: &str, event: &str) -> std::result::Result<(), String> {
    let value = parse_hook_smoke_json(stdout, &format!("Codex {event}"))?;
    validate_codex_smoke_top_level(&value, event)?;
    let hook = value
        .get("hookSpecificOutput")
        .ok_or_else(|| format!("missing hookSpecificOutput in Codex {event} response"))?;
    validate_codex_smoke_event(hook, event)?;
    if hook.get("permissionDecision").is_some() {
        return Err(format!(
            "unsupported hookSpecificOutput.permissionDecision in Codex {event} no-op response"
        ));
    }
    if hook.get("permissionDecisionReason").is_some() {
        return Err(format!(
            "unsupported hookSpecificOutput.permissionDecisionReason in Codex {event} no-op response"
        ));
    }
    Ok(())
}

fn validate_codex_permission_smoke_output(
    stdout: &str,
    event: &str,
    expected: &str,
) -> std::result::Result<(), String> {
    let value = parse_hook_smoke_json(stdout, &format!("Codex {event}"))?;
    validate_codex_smoke_top_level(&value, event)?;
    let hook = value
        .get("hookSpecificOutput")
        .ok_or_else(|| format!("missing hookSpecificOutput in Codex {event} response"))?;
    validate_codex_smoke_event(hook, event)?;
    let decision = hook
        .get("permissionDecision")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            format!("missing hookSpecificOutput.permissionDecision in Codex {event} response")
        })?;
    if decision != expected {
        return Err(format!(
            "unexpected hookSpecificOutput.permissionDecision in Codex {event} response"
        ));
    }
    if expected == "deny" {
        let reason = hook
            .get("permissionDecisionReason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if reason.trim().is_empty() {
            return Err(format!(
                "missing hookSpecificOutput.permissionDecisionReason in Codex {event} response"
            ));
        }
    }
    Ok(())
}

fn validate_codex_smoke_top_level(
    value: &serde_json::Value,
    event: &str,
) -> std::result::Result<(), String> {
    if value.get("suppressOutput").is_some() {
        return Err(format!(
            "unsupported top-level suppressOutput in Codex {event} response"
        ));
    }
    if value.get("decision").is_some() {
        return Err(format!(
            "unsupported top-level decision in Codex {event} response"
        ));
    }
    if value.get("reason").is_some() {
        return Err(format!(
            "unsupported top-level reason in Codex {event} response"
        ));
    }
    Ok(())
}

fn validate_codex_smoke_event(
    hook: &serde_json::Value,
    event: &str,
) -> std::result::Result<(), String> {
    let hook_event = hook
        .get("hookEventName")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if hook_event != event {
        return Err(format!(
            "missing hookSpecificOutput.hookEventName {event} in Codex {event} response"
        ));
    }
    Ok(())
}

fn parse_hook_smoke_json(
    stdout: &str,
    label: &str,
) -> std::result::Result<serde_json::Value, String> {
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .map_err(|error| format!("{label} response stdout is not valid JSON: {error}"))
}

fn cleanup_codex_smoke_files(root: &Path, session_prefix: &str) {
    let dir = codex_hook_dir(root);
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with(session_prefix) {
            let _ = fs::remove_file(path);
        }
    }
}

fn archive_stale_codex_hook_files(root: &Path) -> Result<Vec<String>> {
    let dir = codex_hook_dir(root);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut archived = Vec::new();
    let mut archive_dir = None;
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || !codex_hook_file_is_stale(&path) {
            continue;
        }
        let archive_dir = archive_dir.get_or_insert_with(|| {
            dir.join("archive")
                .join(format!("contract-{}", now_unix_seconds()))
        });
        fs::create_dir_all(&archive_dir)
            .with_context(|| format!("failed to create {}", archive_dir.display()))?;
        let dest = unique_archive_path(
            archive_dir,
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("hook-state.json"),
        );
        fs::rename(&path, &dest).with_context(|| {
            format!("failed to archive stale Codex hook file {}", path.display())
        })?;
        archived.push(format!(
            "archived {} -> {}",
            repo_relative_display(root, &path),
            repo_relative_display(root, &dest)
        ));
    }
    archived.sort();
    Ok(archived)
}

fn unique_archive_path(dir: &Path, file_name: &str) -> PathBuf {
    let mut candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    for index in 1.. {
        candidate = dir.join(format!("{index}-{file_name}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("unbounded archive suffix loop should always return")
}

fn codex_hook_file_is_stale(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.starts_with("callsieve-smoke-")
        || name.starts_with("post-smoke")
        || name.contains("-smoke")
    {
        return true;
    }
    if name.ends_with(".state.json") {
        return fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
            .is_some_and(|value| {
                value
                    .get("stop_blocked")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                    || value
                        .get("violation_seen")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
            });
    }
    if name.ends_with(".trace.json") {
        return fs::read_to_string(path).ok().is_some_and(|content| {
            content.contains("\"PostToolUse\"")
                || content.contains("\"Stop\"")
                || content.contains("post-tool-use")
        });
    }
    false
}

fn codex_hooks_trust_path(root: &Path) -> PathBuf {
    codex_hook_dir(root).join("trust-reviewed.json")
}

fn codex_hook_trust_review(root: &Path, hooks_hash: &str) -> CodexHookTrustReview {
    let trust_path = codex_hooks_trust_path(root);
    let trust_file = repo_relative_display(root, &trust_path);
    let missing = || {
        CodexHookTrustReview {
        status: "manual_review_required".to_string(),
        trust_file: trust_file.clone(),
        hooks_hash: hooks_hash.to_string(),
        reviewed_at: None,
        message: "Review project hooks in Codex with /hooks, then run callsieve codex-hooks trust-ack <repo>.".to_string(),
    }
    };
    if !trust_path.is_file() {
        return missing();
    }
    let Ok(data) = fs::read(&trust_path) else {
        return CodexHookTrustReview {
            status: "manual_review_required".to_string(),
            trust_file,
            hooks_hash: hooks_hash.to_string(),
            reviewed_at: None,
            message:
                "Codex hook trust marker could not be read; review /hooks and refresh trust-ack."
                    .to_string(),
        };
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
        return CodexHookTrustReview {
            status: "manual_review_required".to_string(),
            trust_file,
            hooks_hash: hooks_hash.to_string(),
            reviewed_at: None,
            message:
                "Codex hook trust marker is invalid JSON; review /hooks and refresh trust-ack."
                    .to_string(),
        };
    };
    let reviewed_hash = value
        .get("hooks_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let reviewed_profile = value
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let reviewed_at = value.get("reviewed_at").and_then(serde_json::Value::as_u64);
    if reviewed_hash == hooks_hash && reviewed_profile == CODEX_HOOK_PROFILE {
        CodexHookTrustReview {
            status: "reviewed".to_string(),
            trust_file,
            hooks_hash: hooks_hash.to_string(),
            reviewed_at,
            message: "Codex hook trust marker matches the installed hook file.".to_string(),
        }
    } else {
        CodexHookTrustReview {
            status: "manual_review_required".to_string(),
            trust_file,
            hooks_hash: hooks_hash.to_string(),
            reviewed_at,
            message: "Codex hook trust marker does not match the installed hook file; review /hooks and refresh trust-ack.".to_string(),
        }
    }
}

fn codex_hooks_trust_ack(root: &Path) -> Result<CodexHooksTrustAckOutput> {
    let hooks_path = codex_hooks_path(root);
    let hooks_text = fs::read_to_string(&hooks_path)
        .with_context(|| format!("failed to read {}", hooks_path.display()))?;
    serde_json::from_str::<serde_json::Value>(&hooks_text)
        .with_context(|| format!("failed to parse {}", hooks_path.display()))?;
    let hooks_hash = hook_hash(&hooks_text);
    let trust_path = codex_hooks_trust_path(root);
    if let Some(parent) = trust_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let reviewed_at = now_unix_seconds();
    let value = serde_json::json!({
        "profile": CODEX_HOOK_PROFILE,
        "hooks_file": repo_relative_display(root, &hooks_path),
        "hooks_hash": hooks_hash,
        "reviewed_at": reviewed_at,
        "manual_review": "Human reviewed project hooks in Codex with /hooks before recording this marker."
    });
    fs::write(&trust_path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", trust_path.display()))?;

    Ok(CodexHooksTrustAckOutput {
        command: "codex-hooks trust-ack",
        status: "pass".to_string(),
        root: root_label(root),
        profile: CODEX_HOOK_PROFILE,
        hooks_file: repo_relative_display(root, &hooks_path),
        trust_file: repo_relative_display(root, &trust_path),
        hooks_hash,
        reviewed_at,
        manual_review: "Review project hooks in Codex with /hooks before using this acknowledgement as evidence.",
    })
}

fn codex_hooks_uninstall(root: &Path) -> Result<CodexHooksUninstallOutput> {
    let hooks_path = codex_hooks_path(root);
    let mut files = Vec::new();
    if hooks_path.is_file() {
        fs::remove_file(&hooks_path)
            .with_context(|| format!("failed to remove {}", hooks_path.display()))?;
        files.push(repo_relative_display(root, &hooks_path));
    }
    Ok(CodexHooksUninstallOutput {
        command: "codex-hooks uninstall",
        status: "pass".to_string(),
        root: root_label(root),
        files,
    })
}

fn claude_hooks_install(
    root: &Path,
    strict: bool,
    force: bool,
    limit: usize,
    snippets_per_file: usize,
    lsp: bool,
) -> Result<ClaudeHooksInstallOutput> {
    let index = build_index_output(root, lsp)?;
    let (hooks_file, trace_dir, files) =
        write_claude_hooks_files(root, strict, force, limit, snippets_per_file)?;

    Ok(ClaudeHooksInstallOutput {
        command: "claude-hooks install",
        status: "pass".to_string(),
        root: root_label(root),
        strict,
        hooks_file,
        trace_dir,
        files,
        index,
        first_required_command: format!("callsieve agent-context {} \"<task>\"", root.display()),
        trust_instruction: "Review and trust project hooks in Claude Code with /hooks.",
        policy: "Claude Code lifecycle hooks inject CallSieve context plus local expansion commands and block broad search before context.",
    })
}

fn write_claude_hooks_files(
    root: &Path,
    strict: bool,
    force: bool,
    limit: usize,
    snippets_per_file: usize,
) -> Result<(String, String, Vec<String>)> {
    let mut files = Vec::new();
    let hooks_path = claude_hooks_path(root);
    let config = claude_hooks_json(root, strict, limit, snippets_per_file, force)?;
    let config_text = serde_json::to_string_pretty(&config)?;
    if let Some(parent) = hooks_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&hooks_path, config_text)
        .with_context(|| format!("failed to write {}", hooks_path.display()))?;
    files.push(repo_relative_display(root, &hooks_path));

    let trace_dir = claude_hook_dir(root);
    fs::create_dir_all(&trace_dir)
        .with_context(|| format!("failed to create {}", trace_dir.display()))?;
    files.push(repo_relative_display(root, &trace_dir));
    files.sort();
    files.dedup();
    Ok((
        repo_relative_display(root, &hooks_path),
        repo_relative_display(root, &trace_dir),
        files,
    ))
}

fn claude_hooks_doctor(root: &Path, strict: bool) -> ClaudeHooksDoctorOutput {
    let hooks_path = claude_hooks_path(root);
    let trace_dir = claude_hook_dir(root);
    let content = fs::read_to_string(&hooks_path).ok();
    let parsed = content
        .as_deref()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok());
    let mut checks = Vec::new();
    checks.push(enforce_check(
        "claude_hooks_file",
        hooks_path.is_file(),
        if hooks_path.is_file() {
            "Claude Code settings.local.json exists"
        } else {
            "Claude Code settings.local.json is missing"
        },
    ));
    checks.push(enforce_check(
        "claude_hooks_json",
        parsed.is_some(),
        if parsed.is_some() {
            "Claude Code settings file is valid JSON"
        } else {
            "Claude Code settings file is missing or invalid JSON"
        },
    ));

    for event in CLAUDE_HOOK_EVENTS {
        let installed = parsed
            .as_ref()
            .and_then(|value| value.get("hooks"))
            .and_then(|hooks| hooks.get(event))
            .and_then(serde_json::Value::as_array)
            .is_some_and(|entries| !entries.is_empty());
        checks.push(enforce_check(
            format!("claude_hook_event:{event}"),
            installed,
            if installed {
                format!("{event} hook is installed")
            } else {
                format!("{event} hook is missing")
            },
        ));
    }

    let command_text = content.as_deref().unwrap_or_default();
    checks.push(enforce_check(
        "claude_hook_commands",
        CLAUDE_HOOK_COMMAND_NAMES
            .iter()
            .all(|command| command_text.contains(command)),
        "Claude Code hooks point at CallSieve hook handlers",
    ));
    checks.push(enforce_check(
        "claude_hook_exec_form",
        command_text.contains("\"args\"") && command_text.contains("claude-hook"),
        "Claude Code hooks use command plus args form",
    ));
    checks.push(enforce_check(
        "claude_hook_strict",
        !strict || command_text.contains("--strict"),
        if strict {
            "Claude Code hooks are installed in strict mode"
        } else {
            "Claude Code strict hook mode not required"
        },
    ));
    checks.push(enforce_check(
        "claude_hook_trace_dir",
        trace_dir.is_dir(),
        if trace_dir.is_dir() {
            "Claude Code hook trace directory exists"
        } else {
            "Claude Code hook trace directory is missing"
        },
    ));
    checks.push(check_with_status(
        "claude_hook_trust",
        "warn",
        "Review and trust project hooks in Claude Code with /hooks before relying on enforcement",
    ));

    ClaudeHooksDoctorOutput {
        command: "claude-hooks doctor",
        status: status_from_checks(&checks),
        root: root_label(root),
        strict,
        hooks_file: repo_relative_display(root, &hooks_path),
        trace_dir: repo_relative_display(root, &trace_dir),
        checks,
        trust_instruction: "Review and trust project hooks in Claude Code with /hooks.",
    }
}

fn claude_hooks_uninstall(root: &Path) -> Result<ClaudeHooksUninstallOutput> {
    let hooks_path = claude_hooks_path(root);
    let mut files = Vec::new();
    if hooks_path.is_file() {
        let content = fs::read_to_string(&hooks_path)
            .with_context(|| format!("failed to read {}", hooks_path.display()))?;
        let mut value: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", hooks_path.display()))?;
        remove_claude_hook_entries(&mut value);
        if value
            .get("hooks")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|hooks| hooks.is_empty())
        {
            value
                .as_object_mut()
                .context("Claude settings root must be a JSON object")?
                .remove("hooks");
        }
        if value.as_object().is_some_and(|object| object.is_empty()) {
            fs::remove_file(&hooks_path)
                .with_context(|| format!("failed to remove {}", hooks_path.display()))?;
        } else {
            fs::write(&hooks_path, serde_json::to_vec_pretty(&value)?)
                .with_context(|| format!("failed to write {}", hooks_path.display()))?;
        }
        files.push(repo_relative_display(root, &hooks_path));
    }
    Ok(ClaudeHooksUninstallOutput {
        command: "claude-hooks uninstall",
        status: "pass".to_string(),
        root: root_label(root),
        files,
    })
}

fn client_hooks_install(
    root: &Path,
    client: HookClient,
    strict: bool,
    force: bool,
    limit: usize,
    snippets_per_file: usize,
    lsp: bool,
) -> Result<ClientHooksInstallOutput> {
    let index = build_index_output(root, lsp)?;
    let (hooks_file, trace_dir, files) =
        write_client_hooks_files(root, client, strict, force, limit, snippets_per_file)?;

    Ok(ClientHooksInstallOutput {
        command: format!("{}-hooks install", hook_client_name(client)),
        status: "pass".to_string(),
        root: root_label(root),
        client: agent_client_name(hook_client_agent(client)).to_string(),
        strict,
        hooks_file,
        trace_dir,
        files,
        index,
        first_required_command: format!("callsieve agent-context {} \"<task>\"", root.display()),
        trust_instruction: hook_client_trust_instruction(client).to_string(),
        policy: hook_client_policy(client).to_string(),
    })
}

fn write_client_hooks_files(
    root: &Path,
    client: HookClient,
    strict: bool,
    force: bool,
    limit: usize,
    snippets_per_file: usize,
) -> Result<(String, String, Vec<String>)> {
    let mut files = Vec::new();
    match client {
        HookClient::OpenCode => {
            let plugin_path = client_hooks_path(root, client);
            write_project_file(
                root,
                &plugin_path,
                &opencode_callsieve_plugin(root, strict, limit, snippets_per_file),
                force,
                &mut files,
            )?;
        }
        HookClient::Cline => {
            let manifest_path = client_hooks_path(root, client);
            write_project_file(
                root,
                &manifest_path,
                &cline_hooks_manifest(root, strict, limit, snippets_per_file),
                force,
                &mut files,
            )?;
            for hook_name in CLIENT_HOOK_COMMAND_NAMES {
                for windows in [true, false] {
                    let path = cline_hook_script_path(root, hook_name, windows);
                    let content = if windows {
                        cline_hook_script_ps1(
                            root,
                            hook_name,
                            strict,
                            context_options(hook_name, limit, snippets_per_file),
                        )
                    } else {
                        cline_hook_script_sh(
                            root,
                            hook_name,
                            strict,
                            context_options(hook_name, limit, snippets_per_file),
                        )
                    };
                    write_project_file(root, &path, &content, force, &mut files)?;
                    if !windows {
                        set_executable(&path)?;
                    }
                }
            }
        }
        HookClient::Copilot | HookClient::Antigravity => {
            let hooks_path = client_hooks_path(root, client);
            let config = client_hooks_json(root, client, strict, limit, snippets_per_file);
            write_project_file(
                root,
                &hooks_path,
                &serde_json::to_string_pretty(&config)?,
                force,
                &mut files,
            )?;
        }
    }

    let trace_dir = client_hook_dir(root, client);
    fs::create_dir_all(&trace_dir)
        .with_context(|| format!("failed to create {}", trace_dir.display()))?;
    files.push(repo_relative_display(root, &trace_dir));
    files.sort();
    files.dedup();
    Ok((
        repo_relative_display(root, &client_hooks_path(root, client)),
        repo_relative_display(root, &trace_dir),
        files,
    ))
}

fn client_hooks_doctor(root: &Path, client: HookClient, strict: bool) -> ClientHooksDoctorOutput {
    let hooks_path = client_hooks_path(root, client);
    let trace_dir = client_hook_dir(root, client);
    let content = fs::read_to_string(&hooks_path).ok();
    let mut checks = Vec::new();
    checks.push(enforce_check(
        format!("{}_hooks_file", hook_client_name(client)),
        hooks_path.is_file(),
        if hooks_path.is_file() {
            format!("{} hook file exists", hook_client_display(client))
        } else {
            format!("{} hook file is missing", hook_client_display(client))
        },
    ));

    match client {
        HookClient::OpenCode => {
            let text = content.as_deref().unwrap_or_default();
            checks.push(enforce_check(
                "opencode_plugin_before_hook",
                text.contains("tool.execute.before"),
                "OpenCode plugin registers tool.execute.before",
            ));
            checks.push(enforce_check(
                "opencode_plugin_after_hook",
                text.contains("tool.execute.after"),
                "OpenCode plugin registers tool.execute.after",
            ));
            checks.push(enforce_check(
                "opencode_plugin_session_events",
                text.contains("session.start") && text.contains("session.end"),
                "OpenCode plugin records session events",
            ));
        }
        HookClient::Cline => {
            let parsed = content
                .as_deref()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok());
            checks.push(enforce_check(
                "cline_hooks_json",
                parsed.is_some(),
                "Cline hook manifest is valid JSON",
            ));
            for hook_name in CLIENT_HOOK_COMMAND_NAMES {
                let scripts_installed = cline_hook_script_path(root, hook_name, true).is_file()
                    && cline_hook_script_path(root, hook_name, false).is_file();
                checks.push(enforce_check(
                    format!("cline_hook_script:{hook_name}"),
                    scripts_installed,
                    if scripts_installed {
                        format!("Cline {hook_name} hook scripts are installed")
                    } else {
                        format!("Cline {hook_name} hook scripts are missing")
                    },
                ));
            }
        }
        HookClient::Copilot | HookClient::Antigravity => {
            let parsed = content
                .as_deref()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(content).ok());
            checks.push(enforce_check(
                format!("{}_hooks_json", hook_client_name(client)),
                parsed.is_some(),
                if parsed.is_some() {
                    format!("{} hook file is valid JSON", hook_client_display(client))
                } else {
                    format!(
                        "{} hook file is missing or invalid JSON",
                        hook_client_display(client)
                    )
                },
            ));
            for (event, _) in client_hook_events(client) {
                let installed = parsed
                    .as_ref()
                    .and_then(|value| value.get("hooks"))
                    .and_then(|hooks| hooks.get(*event))
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|entries| !entries.is_empty());
                checks.push(enforce_check(
                    format!("{}_hook_event:{event}", hook_client_name(client)),
                    installed,
                    if installed {
                        format!("{} {event} hook is installed", hook_client_display(client))
                    } else {
                        format!("{} {event} hook is missing", hook_client_display(client))
                    },
                ));
            }
        }
    }

    let command_text = if matches!(client, HookClient::Cline) {
        let mut text = content.unwrap_or_default();
        for hook_name in CLIENT_HOOK_COMMAND_NAMES {
            for windows in [true, false] {
                if let Ok(script) =
                    fs::read_to_string(cline_hook_script_path(root, hook_name, windows))
                {
                    text.push('\n');
                    text.push_str(&script);
                }
            }
        }
        text
    } else {
        content.unwrap_or_default()
    };
    checks.push(enforce_check(
        format!("{}_hook_commands", hook_client_name(client)),
        command_text.contains(hook_client_command_prefix(client)),
        format!(
            "{} hooks point at CallSieve hook handlers",
            hook_client_display(client)
        ),
    ));
    checks.push(enforce_check(
        format!("{}_hook_strict", hook_client_name(client)),
        !strict || command_text.contains("--strict"),
        if strict {
            format!(
                "{} hooks are installed in strict mode",
                hook_client_display(client)
            )
        } else {
            format!("{} strict hooks are optional", hook_client_display(client))
        },
    ));
    checks.push(enforce_check(
        format!("{}_hook_trace_dir", hook_client_name(client)),
        trace_dir.is_dir(),
        if trace_dir.is_dir() {
            format!(
                "{} hook trace directory exists",
                hook_client_display(client)
            )
        } else {
            format!(
                "{} hook trace directory is missing",
                hook_client_display(client)
            )
        },
    ));
    checks.push(check_with_status(
        format!("{}_hook_trust", hook_client_name(client)),
        "warn",
        hook_client_trust_instruction(client),
    ));

    ClientHooksDoctorOutput {
        command: format!("{}-hooks doctor", hook_client_name(client)),
        status: status_from_checks(&checks),
        root: root_label(root),
        client: hook_client_name(client).to_string(),
        strict,
        hooks_file: repo_relative_display(root, &hooks_path),
        trace_dir: repo_relative_display(root, &trace_dir),
        checks,
        trust_instruction: hook_client_trust_instruction(client).to_string(),
    }
}

fn client_hooks_uninstall(root: &Path, client: HookClient) -> Result<ClientHooksUninstallOutput> {
    let mut files = Vec::new();
    let hooks_path = client_hooks_path(root, client);
    if hooks_path.is_file() {
        fs::remove_file(&hooks_path)
            .with_context(|| format!("failed to remove {}", hooks_path.display()))?;
        files.push(repo_relative_display(root, &hooks_path));
    }
    if matches!(client, HookClient::Cline) {
        for hook_name in CLIENT_HOOK_COMMAND_NAMES {
            for windows in [true, false] {
                let path = cline_hook_script_path(root, hook_name, windows);
                if path.is_file() {
                    fs::remove_file(&path)
                        .with_context(|| format!("failed to remove {}", path.display()))?;
                    files.push(repo_relative_display(root, &path));
                }
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(ClientHooksUninstallOutput {
        command: format!("{}-hooks uninstall", hook_client_name(client)),
        status: "pass".to_string(),
        root: root_label(root),
        client: hook_client_name(client).to_string(),
        files,
    })
}

fn hook_prompt_should_inject_context(prompt: &str) -> bool {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return false;
    }

    let tokens = query::formatter::tokenize(prompt);
    if tokens.is_empty() {
        return false;
    }

    let normalized = tokens.join(" ");
    let low_signal_prompts = [
        "ok",
        "okay",
        "yes",
        "no",
        "yep",
        "nope",
        "thanks",
        "thank you",
        "done",
        "did it",
        "i did it",
        "got it",
        "all set",
        "sounds good",
        "great",
        "nice",
        "cool",
        "perfect",
        "confirmed",
        "it worked",
        "that worked",
    ];
    if low_signal_prompts.contains(&normalized.as_str()) {
        return false;
    }

    let codebase_intent_terms = [
        "add",
        "benchmark",
        "bug",
        "build",
        "cargo",
        "change",
        "class",
        "cli",
        "code",
        "compile",
        "daemon",
        "debug",
        "delete",
        "dependencies",
        "dependency",
        "doc",
        "docs",
        "error",
        "explain",
        "fail",
        "failing",
        "failure",
        "file",
        "files",
        "find",
        "fix",
        "function",
        "hook",
        "hooks",
        "how",
        "implement",
        "investigate",
        "issue",
        "mcp",
        "module",
        "notes",
        "python",
        "readme",
        "refactor",
        "release",
        "repo",
        "repository",
        "review",
        "rust",
        "shim",
        "symbol",
        "test",
        "tests",
        "trace",
        "typescript",
        "update",
        "where",
        "why",
    ];
    if tokens
        .iter()
        .any(|token| codebase_intent_terms.contains(&token.as_str()))
    {
        return true;
    }

    tokens.len() >= 4
}

fn effective_task_for_retrieval(root: &Path, task: &str) -> String {
    let task = task.trim();
    if !hook_prompt_is_anaphoric_followup(task) {
        return task.to_string();
    }
    query::latest_task_memory_task(root)
        .map(|previous| format!("{}\nFollow-up: {task}", previous.trim()))
        .unwrap_or_else(|| task.to_string())
}

fn hook_effective_prompt_for_retrieval(
    root: &Path,
    state: &CodexHookState,
    prompt: &str,
) -> String {
    let prompt = prompt.trim();
    let previous = state.last_prompt.trim();
    if !hook_prompt_is_anaphoric_followup(prompt) {
        return prompt.to_string();
    }
    if !previous.is_empty() {
        return format!("{previous}\nFollow-up: {prompt}");
    }
    effective_task_for_retrieval(root, prompt)
}

fn hook_prompt_is_anaphoric_followup(prompt: &str) -> bool {
    let tokens = query::formatter::tokenize(prompt);
    if tokens.is_empty() {
        return false;
    }
    let normalized = tokens.join(" ");
    let direct_followups = [
        "do it",
        "do that",
        "fix it",
        "fix that",
        "keep going",
        "lets do it",
        "make it better",
        "what weak",
    ];
    direct_followups.contains(&normalized.as_str())
        || (tokens.iter().any(|token| token == "fix")
            && prompt.chars().any(|character| character.is_ascii_digit()))
}

fn hook_skipped_user_prompt_submit_response() -> serde_json::Value {
    serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit"
        }
    })
}

fn hook_context_intro(client: Option<&str>) -> String {
    match client {
        Some(client) => format!(
            "CallSieve context ready for {client}. Read these files first. Use broad search only if this packet is insufficient."
        ),
        None => "CallSieve context ready. Read these files first. Use broad search only if this packet is insufficient.".to_string(),
    }
}

fn hook_missing_index_message(root: &Path, retrieval_prompt: &str) -> String {
    format!(
        "CallSieve index is missing or stale for {}. Run `callsieve index {}` or `callsieve agent-context {} {:?}` before broad search or repeated file reads.",
        root.display(),
        root.display(),
        root.display(),
        retrieval_prompt
    )
}

fn hook_injected_context_trace_event(
    root: &Path,
    retrieval_prompt: &str,
    context_selected_files: Vec<String>,
    tokens: usize,
) -> serde_json::Value {
    serde_json::json!({
        "timestamp": now_unix_seconds(),
        "command": first_required_context_command(root, retrieval_prompt),
        "files_read": [],
        "context_selected_files": context_selected_files,
        "tokens": tokens,
        "classification": "callsieve_context",
        "phase": "callsieve",
        "hook_event": "UserPromptSubmit"
    })
}

fn codex_hook_user_prompt_submit(
    root: &Path,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
) -> Result<serde_json::Value> {
    let input = read_codex_hook_input()?;
    let prompt = hook_string_field(&input, &["prompt"]).unwrap_or_default();
    let session_id = hook_session_id(&input);
    let turn_id = hook_turn_id(&input);
    let mut state = load_codex_hook_state(root, &session_id);
    state.turn_id = turn_id;
    state.strict = strict;
    let retrieval_prompt = hook_effective_prompt_for_retrieval(root, &state, &prompt);
    let recovered_followup = retrieval_prompt.trim() != prompt.trim();
    let should_inject = hook_prompt_should_inject_context(&prompt) || recovered_followup;
    state.last_prompt_hash = hook_hash(&prompt);
    state.updated_at = now_unix_seconds();
    if !should_inject {
        state.context_seen = false;
        state.selected_files.clear();
        save_codex_hook_state(root, &state)?;
        return Ok(hook_skipped_user_prompt_submit_response());
    }
    state.last_prompt = prompt;

    let additional_context = match load_fresh_index_timed(root) {
        Ok((index, index_load_ms, refreshed)) => {
            let mut context = query::build_context(
                root,
                &index,
                &retrieval_prompt,
                limit,
                snippets_per_file,
                true,
            )?;
            context.add_index_load_time(index_load_ms);
            if refreshed {
                context.add_warning("rebuilt missing or stale CallSieve index before context");
            }
            let context_value = compact_context_value_for_prompt(&context)?;
            let files = context_read_first_files(&context_value);
            let tokens = serde_json::to_string(&context_value)
                .map(|json| json.len().div_ceil(4))
                .unwrap_or_default();
            note_packet_served(&mut state, root, &retrieval_prompt, "codex");
            state.context_seen = true;
            state.selected_files = files.clone();
            state.context_packets += 1;
            state.context_tokens += tokens;
            state.context_files += files.len();
            let local_first_expansion =
                local_first_expansion_for_context_value(root, &context_value);
            append_codex_hook_trace_event(
                root,
                &state,
                &retrieval_prompt,
                hook_injected_context_trace_event(root, &retrieval_prompt, files, tokens),
            )?;
            format!(
                "{}\n\n{}",
                hook_context_intro(None),
                context_markdown(
                    &context_value,
                    "grep_only_if_context_is_insufficient",
                    local_first_expansion.as_ref(),
                )
            )
        }
        Err(_) => {
            state.context_seen = false;
            hook_missing_index_message(root, &retrieval_prompt)
        }
    };

    save_codex_hook_state(root, &state)?;
    Ok(serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": additional_context
        }
    }))
}

fn codex_hook_pre_tool_use(root: &Path, strict: bool) -> Result<serde_json::Value> {
    let input = read_codex_hook_input()?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(&input, &["prompt", "task"]).unwrap_or_default();
    let mut state = load_codex_hook_state(root, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);

    if is_callsieve_context_command_local(&command) {
        state.context_seen = true;
        append_codex_hook_trace_event(
            root,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, false),
        )?;
        save_codex_hook_state(root, &state)?;
        return Ok(codex_hook_noop_response("PreToolUse"));
    }

    if codex_hook_should_deny(&state, strict, &command) {
        state.violation_seen = true;
        append_codex_hook_trace_event(
            root,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, true),
        )?;
        save_codex_hook_state(root, &state)?;
        return Ok(codex_hook_permission_response(
            "PreToolUse",
            "deny",
            &codex_hook_denial_reason(&command, strict),
        ));
    }

    record_confirmed_reads(&mut state, root, &command);
    save_codex_hook_state(root, &state)?;
    Ok(codex_hook_noop_response("PreToolUse"))
}

fn codex_hook_post_tool_use(_root: &Path, _strict: bool) -> Result<()> {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    Ok(())
}

fn codex_hook_permission_request(root: &Path, strict: bool) -> Result<serde_json::Value> {
    let input = read_codex_hook_input()?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(&input, &["prompt", "task"]).unwrap_or_default();
    let mut state = load_codex_hook_state(root, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);
    if codex_hook_should_deny(&state, strict, &command) {
        state.violation_seen = true;
        append_codex_hook_trace_event(
            root,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, true),
        )?;
        save_codex_hook_state(root, &state)?;
        return Ok(codex_hook_permission_response(
            "PermissionRequest",
            "deny",
            &codex_hook_denial_reason(&command, strict),
        ));
    }
    save_codex_hook_state(root, &state)?;
    Ok(codex_hook_noop_response("PermissionRequest"))
}

fn codex_hook_stop(root: &Path, _strict: bool) -> Result<()> {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    // The Codex Stop protocol requires empty stdout (a smoke check enforces
    // it), so the session folds into task memory silently.
    let parsed: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let session_id = hook_session_id(&parsed);
    let state = load_codex_hook_state(root, &session_id);
    let _ = fold_session_into_task_memory(root, &state, "codex");
    Ok(())
}

fn claude_hook_user_prompt_submit(
    root: &Path,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
) -> Result<serde_json::Value> {
    let input = read_claude_hook_input()?;
    let prompt =
        hook_string_field(&input, &["prompt", "user_prompt", "userPrompt"]).unwrap_or_default();
    let session_id = hook_session_id(&input);
    let turn_id = hook_turn_id(&input);
    let mut state = load_claude_hook_state(root, &session_id);
    state.turn_id = turn_id;
    state.strict = strict;
    let retrieval_prompt = hook_effective_prompt_for_retrieval(root, &state, &prompt);
    let recovered_followup = retrieval_prompt.trim() != prompt.trim();
    let should_inject = hook_prompt_should_inject_context(&prompt) || recovered_followup;
    state.last_prompt_hash = hook_hash(&prompt);
    state.updated_at = now_unix_seconds();
    if !should_inject {
        state.context_seen = false;
        state.selected_files.clear();
        save_claude_hook_state(root, &state)?;
        return Ok(hook_skipped_user_prompt_submit_response());
    }
    state.last_prompt = prompt;

    let additional_context = match load_fresh_index_timed(root) {
        Ok((index, index_load_ms, refreshed)) => {
            let mut context = query::build_context(
                root,
                &index,
                &retrieval_prompt,
                limit,
                snippets_per_file,
                true,
            )?;
            context.add_index_load_time(index_load_ms);
            if refreshed {
                context.add_warning("rebuilt missing or stale CallSieve index before context");
            }
            let context_value = compact_context_value_for_prompt(&context)?;
            let files = context_read_first_files(&context_value);
            let tokens = serde_json::to_string(&context_value)
                .map(|json| json.len().div_ceil(4))
                .unwrap_or_default();
            note_packet_served(&mut state, root, &retrieval_prompt, "claude");
            state.context_seen = true;
            state.selected_files = files.clone();
            state.context_packets += 1;
            state.context_tokens += tokens;
            state.context_files += files.len();
            let local_first_expansion =
                local_first_expansion_for_context_value(root, &context_value);
            append_claude_hook_trace_event(
                root,
                &state,
                &retrieval_prompt,
                hook_injected_context_trace_event(root, &retrieval_prompt, files, tokens),
            )?;
            format!(
                "{}\n\n{}",
                hook_context_intro(Some("Claude")),
                context_markdown(
                    &context_value,
                    "grep_only_if_context_is_insufficient",
                    local_first_expansion.as_ref(),
                )
            )
        }
        Err(_) => {
            state.context_seen = false;
            hook_missing_index_message(root, &retrieval_prompt)
        }
    };

    save_claude_hook_state(root, &state)?;
    Ok(serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": additional_context
        }
    }))
}

fn claude_hook_pre_tool_use(root: &Path, strict: bool) -> Result<serde_json::Value> {
    let input = read_claude_hook_input()?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(&input, &["prompt", "task", "user_prompt", "userPrompt"])
        .unwrap_or_default();
    let mut state = load_claude_hook_state(root, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);

    if is_callsieve_context_command_local(&command) {
        state.context_seen = true;
        append_claude_hook_trace_event(
            root,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, false),
        )?;
        save_claude_hook_state(root, &state)?;
        return Ok(claude_hook_pre_tool_response(
            "PreToolUse",
            "allow",
            "CallSieve context is allowed.",
        ));
    }

    if codex_hook_should_deny(&state, strict, &command) {
        state.violation_seen = true;
        append_claude_hook_trace_event(
            root,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, true),
        )?;
        save_claude_hook_state(root, &state)?;
        return Ok(claude_hook_pre_tool_response(
            "PreToolUse",
            "deny",
            &codex_hook_denial_reason(&command, strict),
        ));
    }

    save_claude_hook_state(root, &state)?;
    Ok(claude_hook_pre_tool_response(
        "PreToolUse",
        "allow",
        "Tool use allowed by CallSieve hook policy.",
    ))
}

fn claude_hook_post_tool_use(root: &Path, strict: bool) -> Result<serde_json::Value> {
    let input = read_claude_hook_input()?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(&input, &["prompt", "task", "user_prompt", "userPrompt"])
        .unwrap_or_default();
    let mut state = load_claude_hook_state(root, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);
    if is_callsieve_context_command_local(&command) {
        state.context_seen = true;
    }
    record_confirmed_reads(&mut state, root, &command);
    let edited_file = hook_edited_file(&input, root);
    let edit_impact = edited_file
        .as_deref()
        .and_then(|edited| hook_edit_impact_context(root, edited));
    let mut trace_event = codex_hook_trace_event(&input, &command, false);
    if edited_file.is_some() {
        // Classify by what the agent did (an edit), not by whether the index
        // had graph edges for it.
        trace_event["classification"] = serde_json::Value::String("edit_impact".to_string());
    }
    append_claude_hook_trace_event(root, &state, &task, trace_event)?;
    save_claude_hook_state(root, &state)?;
    let mut response = serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse"
        }
    });
    if let Some(impact) = edit_impact {
        response["hookSpecificOutput"]["additionalContext"] = serde_json::Value::String(impact);
    }
    Ok(response)
}

fn claude_hook_permission_request(root: &Path, strict: bool) -> Result<serde_json::Value> {
    let input = read_claude_hook_input()?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(&input, &["prompt", "task", "user_prompt", "userPrompt"])
        .unwrap_or_default();
    let mut state = load_claude_hook_state(root, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);
    if codex_hook_should_deny(&state, strict, &command) {
        state.violation_seen = true;
        append_claude_hook_trace_event(
            root,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, true),
        )?;
        save_claude_hook_state(root, &state)?;
        return Ok(claude_hook_permission_request_response(
            "deny",
            &codex_hook_denial_reason(&command, strict),
        ));
    }
    save_claude_hook_state(root, &state)?;
    Ok(serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest"
        }
    }))
}

fn claude_hook_stop(root: &Path, _strict: bool) -> Result<serde_json::Value> {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let parsed: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let session_id = hook_session_id(&parsed);
    let state = load_claude_hook_state(root, &session_id);
    let learned = fold_session_into_task_memory(root, &state, "claude");
    let mut response = serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "Stop"
        }
    });
    if let Some(message) = hook_session_summary_message(&state, learned) {
        response["systemMessage"] = serde_json::Value::String(message);
        response["suppressOutput"] = serde_json::Value::Bool(false);
    }
    Ok(response)
}

/// Learn from the observed session: files the agent actually read after
/// context become confirmed task-memory associations. Errors are swallowed —
/// learning must never break a Stop hook.
/// A new packet starts a new learning window: fold reads confirmed under the
/// previous retrieval task into memory first, so a session that switches
/// topics never attributes early reads to the final prompt.
fn note_packet_served(state: &mut CodexHookState, root: &Path, retrieval_task: &str, client: &str) {
    if !state.confirmed_reads.is_empty()
        && !state.last_retrieval_task.is_empty()
        && state.last_retrieval_task != retrieval_task
    {
        let _ = query::confirm_task_memory_reads(
            root,
            &state.last_retrieval_task,
            &state.selected_files,
            &state.confirmed_reads,
            client,
            now_unix_seconds(),
        );
        state.confirmed_reads.clear();
    }
    state.last_retrieval_task = retrieval_task.to_string();
}

fn fold_session_into_task_memory(root: &Path, state: &CodexHookState, client: &str) -> usize {
    if state.context_packets == 0 || state.confirmed_reads.is_empty() {
        return 0;
    }
    let task = if state.last_retrieval_task.is_empty() {
        &state.last_prompt
    } else {
        &state.last_retrieval_task
    };
    query::confirm_task_memory_reads(
        root,
        task,
        &state.selected_files,
        &state.confirmed_reads,
        client,
        now_unix_seconds(),
    )
    .unwrap_or(0)
}

/// Factual session summary for Stop hooks. Reports only locally observed
/// counts; estimated or comparative savings claims stay gated behind audited
/// observed-session reports.
/// Compact edit-impact note for PostToolUse additionalContext. Only emitted
/// when the index already knows the file and the impact is non-trivial;
/// never triggers an index rebuild (the pre-edit graph is the right one).
fn hook_edit_impact_context(root: &Path, edited: &str) -> Option<String> {
    let index = store::json_store::load_index(root).ok()?;
    let impact = query::edit_impact_for_file(&index, edited)?;
    if impact.callers.is_empty() && impact.tests.is_empty() {
        return None;
    }
    let mut message = format!(
        "CallSieve impact: editing {} (risk: {}).",
        impact.file, impact.risk
    );
    if !impact.callers.is_empty() {
        message.push_str(&format!(" Called by: {}.", impact.callers.join(", ")));
    }
    if !impact.tests.is_empty() {
        message.push_str(&format!(" Related tests: {}.", impact.tests.join(", ")));
    }
    message.push_str(&format!(
        " Detail: callsieve focus {} --file {}",
        root.display(),
        impact.file
    ));
    Some(message)
}

/// Factual session summary for Stop hooks. Reports only locally observed
/// counts; estimated or comparative savings claims stay gated behind audited
/// observed-session reports (a test pins the absence of "saved").
fn hook_session_summary_message(state: &CodexHookState, learned: usize) -> Option<String> {
    if state.context_packets == 0 {
        return None;
    }
    let learned_note = if learned > 0 {
        format!(
            " Learned {} read association{} for future retrieval.",
            learned,
            if learned == 1 { "" } else { "s" }
        )
    } else {
        String::new()
    };
    Some(format!(
        "CallSieve: {} context packet{} (~{} tokens, {} read-first file{}) served this session; local retrieval spent 0 AI model tokens.{} Receipt: `callsieve receipt <repo> --latest`.",
        state.context_packets,
        if state.context_packets == 1 { "" } else { "s" },
        state.context_tokens,
        state.context_files,
        if state.context_files == 1 { "" } else { "s" },
        learned_note,
    ))
}

fn claude_hook_permission_request_response(decision: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": {
                "behavior": decision,
                "message": reason
            }
        }
    })
}

fn client_hook_user_prompt_submit(
    root: &Path,
    client: HookClient,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
) -> Result<serde_json::Value> {
    let input = read_hook_input(hook_client_display(client))?;
    let prompt = hook_string_field(
        &input,
        &["prompt", "user_prompt", "userPrompt", "message", "task"],
    )
    .unwrap_or_default();
    let session_id = hook_session_id(&input);
    let turn_id = hook_turn_id(&input);
    let mut state = load_client_hook_state(root, client, &session_id);
    state.turn_id = turn_id;
    state.strict = strict;
    let retrieval_prompt = hook_effective_prompt_for_retrieval(root, &state, &prompt);
    let recovered_followup = retrieval_prompt.trim() != prompt.trim();
    let should_inject = hook_prompt_should_inject_context(&prompt) || recovered_followup;
    state.last_prompt_hash = hook_hash(&prompt);
    state.updated_at = now_unix_seconds();
    if !should_inject {
        state.context_seen = false;
        state.selected_files.clear();
        save_client_hook_state(root, client, &state)?;
        return Ok(hook_skipped_user_prompt_submit_response());
    }
    state.last_prompt = prompt;

    let additional_context = match load_fresh_index_timed(root) {
        Ok((index, index_load_ms, refreshed)) => {
            let mut context = query::build_context(
                root,
                &index,
                &retrieval_prompt,
                limit,
                snippets_per_file,
                true,
            )?;
            context.add_index_load_time(index_load_ms);
            if refreshed {
                context.add_warning("rebuilt missing or stale CallSieve index before context");
            }
            let context_value = compact_context_value_for_prompt(&context)?;
            let files = context_read_first_files(&context_value);
            let tokens = serde_json::to_string(&context_value)
                .map(|json| json.len().div_ceil(4))
                .unwrap_or_default();
            note_packet_served(
                &mut state,
                root,
                &retrieval_prompt,
                hook_client_name(client),
            );
            state.context_seen = true;
            state.selected_files = files.clone();
            state.context_packets += 1;
            state.context_tokens += tokens;
            state.context_files += files.len();
            let local_first_expansion =
                local_first_expansion_for_context_value(root, &context_value);
            append_client_hook_trace_event(
                root,
                client,
                &state,
                &retrieval_prompt,
                hook_injected_context_trace_event(root, &retrieval_prompt, files, tokens),
            )?;
            format!(
                "{}\n\n{}",
                hook_context_intro(Some(hook_client_display(client))),
                context_markdown(
                    &context_value,
                    "grep_only_if_context_is_insufficient",
                    local_first_expansion.as_ref(),
                )
            )
        }
        Err(_) => {
            state.context_seen = false;
            hook_missing_index_message(root, &retrieval_prompt)
        }
    };

    save_client_hook_state(root, client, &state)?;
    Ok(serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "UserPromptSubmit",
            "additionalContext": additional_context
        }
    }))
}

fn client_hook_pre_tool_use(
    root: &Path,
    client: HookClient,
    strict: bool,
) -> Result<serde_json::Value> {
    let input = read_hook_input(hook_client_display(client))?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(
        &input,
        &["prompt", "task", "user_prompt", "userPrompt", "message"],
    )
    .unwrap_or_default();
    let mut state = load_client_hook_state(root, client, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);

    if is_callsieve_context_command_local(&command) {
        state.context_seen = true;
        append_client_hook_trace_event(
            root,
            client,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, false),
        )?;
        save_client_hook_state(root, client, &state)?;
        return Ok(client_hook_permission_response(
            "PreToolUse",
            "allow",
            "CallSieve context is allowed.",
        ));
    }

    if codex_hook_should_deny(&state, strict, &command) {
        state.violation_seen = true;
        append_client_hook_trace_event(
            root,
            client,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, true),
        )?;
        save_client_hook_state(root, client, &state)?;
        return Ok(client_hook_permission_response(
            "PreToolUse",
            "deny",
            &codex_hook_denial_reason(&command, strict),
        ));
    }

    save_client_hook_state(root, client, &state)?;
    Ok(client_hook_permission_response(
        "PreToolUse",
        "allow",
        "Tool use allowed by CallSieve hook policy.",
    ))
}

fn client_hook_post_tool_use(
    root: &Path,
    client: HookClient,
    strict: bool,
) -> Result<serde_json::Value> {
    let input = read_hook_input(hook_client_display(client))?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(
        &input,
        &["prompt", "task", "user_prompt", "userPrompt", "message"],
    )
    .unwrap_or_default();
    let mut state = load_client_hook_state(root, client, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);
    if is_callsieve_context_command_local(&command) {
        state.context_seen = true;
    }
    record_confirmed_reads(&mut state, root, &command);
    let edited_file = hook_edited_file(&input, root);
    let edit_impact = edited_file
        .as_deref()
        .and_then(|edited| hook_edit_impact_context(root, edited));
    let mut trace_event = codex_hook_trace_event(&input, &command, false);
    if edited_file.is_some() {
        trace_event["classification"] = serde_json::Value::String("edit_impact".to_string());
    }
    append_client_hook_trace_event(root, client, &state, &task, trace_event)?;
    save_client_hook_state(root, client, &state)?;
    let mut response = serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse"
        }
    });
    if let Some(impact) = edit_impact {
        response["hookSpecificOutput"]["additionalContext"] = serde_json::Value::String(impact);
    }
    Ok(response)
}

fn client_hook_permission_request(
    root: &Path,
    client: HookClient,
    strict: bool,
) -> Result<serde_json::Value> {
    let input = read_hook_input(hook_client_display(client))?;
    let session_id = hook_session_id(&input);
    let task = hook_string_field(
        &input,
        &["prompt", "task", "user_prompt", "userPrompt", "message"],
    )
    .unwrap_or_default();
    let mut state = load_client_hook_state(root, client, &session_id);
    state.strict = strict;
    let command = hook_tool_command(&input);
    if codex_hook_should_deny(&state, strict, &command) {
        state.violation_seen = true;
        append_client_hook_trace_event(
            root,
            client,
            &state,
            &task,
            codex_hook_trace_event(&input, &command, true),
        )?;
        save_client_hook_state(root, client, &state)?;
        return Ok(client_hook_permission_response(
            "PermissionRequest",
            "deny",
            &codex_hook_denial_reason(&command, strict),
        ));
    }
    save_client_hook_state(root, client, &state)?;
    Ok(serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest"
        }
    }))
}

fn client_hook_stop(root: &Path, client: HookClient, _strict: bool) -> Result<serde_json::Value> {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let parsed: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let session_id = hook_session_id(&parsed);
    let state = load_client_hook_state(root, client, &session_id);
    let learned = fold_session_into_task_memory(root, &state, hook_client_name(client));
    let mut response = serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": "Stop"
        }
    });
    if let Some(message) = hook_session_summary_message(&state, learned) {
        response["systemMessage"] = serde_json::Value::String(message);
        response["suppressOutput"] = serde_json::Value::Bool(false);
    }
    Ok(response)
}

fn install_hook(
    root: &Path,
    client: AgentClient,
    strict: bool,
    force: bool,
    lsp: bool,
) -> Result<HookInstallOutput> {
    let setup = setup_agent(client, root, force)?;
    let first_required_command = setup.first_required_command.clone();
    let shim = install_shim(root, force, strict)?;
    let mut launchers = write_hook_launchers(root, &first_required_command, force)?;
    if matches!(client, AgentClient::Codex) {
        launchers.extend(write_codex_launchers(root, &first_required_command, force)?);
    }
    let codex_hook_files = if matches!(client, AgentClient::Codex) {
        Some(write_codex_hooks_files(root, strict, force, 6, 1)?)
    } else {
        None
    };
    let claude_hook_files = if matches!(client, AgentClient::Claude) {
        Some(write_claude_hooks_files(root, strict, force, 6, 1)?)
    } else {
        None
    };
    let client_hook_kind = hook_client_for_agent(client);
    let client_hook_files = if let Some(hook_client) = client_hook_kind {
        Some(write_client_hooks_files(
            root,
            hook_client,
            strict,
            force,
            6,
            1,
        )?)
    } else {
        None
    };

    let index = build_index_output(root, lsp)?;

    let codex_hooks = codex_hook_files.map(|(hooks_file, trace_dir, files)| {
        CodexHooksInstallOutput {
            command: "codex-hooks install",
            status: "pass".to_string(),
            root: root_label(root),
            profile: CODEX_HOOK_PROFILE,
            strict,
            hooks_file,
            trace_dir,
            files,
            index: index.clone(),
            first_required_command: first_required_command.clone(),
            trust_instruction: "Review and trust project hooks in Codex with /hooks.",
            policy: "Codex lifecycle hooks inject CallSieve context and block broad search before context.",
        }
    });
    let claude_hooks = claude_hook_files.map(|(hooks_file, trace_dir, files)| {
        ClaudeHooksInstallOutput {
            command: "claude-hooks install",
            status: "pass".to_string(),
            root: root_label(root),
            strict,
            hooks_file,
            trace_dir,
            files,
            index: index.clone(),
            first_required_command: first_required_command.clone(),
            trust_instruction: "Review and trust project hooks in Claude Code with /hooks.",
            policy: "Claude Code lifecycle hooks inject CallSieve context and block broad search before context.",
        }
    });
    let client_hooks = client_hook_files.map(|(hooks_file, trace_dir, files)| {
        let hook_client = client_hook_kind.expect("client hook kind should exist");
        ClientHooksInstallOutput {
            command: format!("{}-hooks install", hook_client_name(hook_client)),
            status: "pass".to_string(),
            root: root_label(root),
            client: hook_client_name(hook_client).to_string(),
            strict,
            hooks_file,
            trace_dir,
            files,
            index: index.clone(),
            first_required_command: first_required_command.clone(),
            trust_instruction: hook_client_trust_instruction(hook_client).to_string(),
            policy: hook_client_policy(hook_client).to_string(),
        }
    });

    Ok(HookInstallOutput {
        command: "hook install",
        status: "pass".to_string(),
        root: root_label(root),
        client: agent_client_name(client).to_string(),
        strict,
        index,
        setup,
        path_instruction: shim.path_instruction.clone(),
        shim,
        codex_hooks,
        claude_hooks,
        client_hooks,
        launchers,
        first_required_command,
        policy: "repo-local hook only; launchers prepend .callsieve/bin for that process and do not mutate global PATH",
        warnings: agent_client_warnings_for_root(client, root),
    })
}

fn hook_doctor(root: &Path) -> HookDoctorOutput {
    let launchers = hook_launcher_paths(root);
    let launchers_installed = launchers.iter().all(|path| path.is_file());
    let mut integrations = Vec::new();
    let mut checks = vec![enforce_check(
        "hook_launchers",
        launchers_installed,
        if launchers_installed {
            "repo-local hook launchers exist"
        } else {
            "run callsieve hook install to create repo-local launchers"
        },
    )];
    let mut shim = shim_doctor(root);
    if launchers_installed {
        for check in &mut shim.checks {
            if check.check == "path_contains_shim_dir" {
                check.status = "pass".to_string();
                check.message =
                    "hook launchers prepend shim bin directory for launched agents".to_string();
            }
        }
        shim.status = if shim.checks.iter().all(|check| check.status == "pass") {
            "pass"
        } else {
            "fail"
        }
        .to_string();
    }
    checks.extend(shim.checks.iter().cloned());

    if codex_hooks_path(root).is_file() {
        let doctor = codex_hooks_doctor(root, false);
        checks.push(enforce_check(
            "codex_hooks",
            doctor.status == "pass",
            if doctor.status == "pass" {
                "Codex lifecycle hooks are installed"
            } else {
                "Codex lifecycle hooks are missing or stale"
            },
        ));
        integrations.push(hook_doctor_integration_summary(
            "codex",
            &doctor.status,
            &doctor.hooks_file,
            &doctor.checks,
        ));
    }

    if claude_hooks_path(root).is_file() {
        let doctor = claude_hooks_doctor(root, false);
        checks.push(enforce_check(
            "claude_hooks",
            doctor.status == "pass",
            if doctor.status == "pass" {
                "Claude Code lifecycle hooks are installed"
            } else {
                "Claude Code lifecycle hooks are missing or stale"
            },
        ));
        integrations.push(hook_doctor_integration_summary(
            "claude",
            &doctor.status,
            &doctor.hooks_file,
            &doctor.checks,
        ));
    }

    for client in [
        HookClient::Copilot,
        HookClient::OpenCode,
        HookClient::Antigravity,
        HookClient::Cline,
    ] {
        if client_hooks_path(root, client).is_file() {
            let doctor = client_hooks_doctor(root, client, false);
            checks.push(enforce_check(
                format!("{}_hooks", hook_client_name(client)),
                doctor.status == "pass",
                if doctor.status == "pass" {
                    format!("{} hooks are installed", hook_client_display(client))
                } else {
                    format!("{} hooks are missing or stale", hook_client_display(client))
                },
            ));
            integrations.push(hook_doctor_integration_summary(
                hook_client_name(client),
                &doctor.status,
                &doctor.hooks_file,
                &doctor.checks,
            ));
        }
    }

    let status = status_from_checks(&checks);

    HookDoctorOutput {
        command: "hook doctor",
        status,
        root: root_label(root),
        checks,
        integrations,
    }
}

fn hook_doctor_integration_summary(
    client: &str,
    status: &str,
    hooks_file: &str,
    checks: &[EnforceCheck],
) -> HookDoctorIntegration {
    let mut events = checks
        .iter()
        .filter(|check| check.status == "pass")
        .filter_map(|check| {
            check
                .check
                .split_once("hook_event:")
                .map(|(_, event)| event.to_string())
        })
        .collect::<Vec<_>>();
    events.sort();
    events.dedup();
    HookDoctorIntegration {
        client: client.to_string(),
        status: status.to_string(),
        profile: if client == "codex" {
            CODEX_HOOK_PROFILE.to_string()
        } else {
            "lifecycle".to_string()
        },
        hooks_file: hooks_file.to_string(),
        events,
    }
}

fn uninstall_hook(root: &Path) -> Result<ShimOutput> {
    let mut output = uninstall_shim(root)?;
    for path in hook_launcher_paths(root) {
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            output.files.push(path.display().to_string());
        }
    }
    for path in codex_launcher_paths(root) {
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            output.files.push(path.display().to_string());
        }
    }
    let codex = codex_hooks_uninstall(root)?;
    output.files.extend(codex.files);
    let claude = claude_hooks_uninstall(root)?;
    output.files.extend(claude.files);
    for hook_client in [
        HookClient::Copilot,
        HookClient::OpenCode,
        HookClient::Antigravity,
        HookClient::Cline,
    ] {
        let removed = client_hooks_uninstall(root, hook_client)?;
        output.files.extend(removed.files);
    }
    output.command = "hook uninstall";
    Ok(output)
}

const CODEX_HOOK_EVENTS: &[&str] = &["UserPromptSubmit", "PreToolUse", "PermissionRequest"];

const CODEX_HOOK_COMMAND_NAMES: &[&str] =
    &["user-prompt-submit", "pre-tool-use", "permission-request"];
const CODEX_HOOK_PROFILE: &str = "slim";

const CLAUDE_HOOK_EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
];

const CLAUDE_HOOK_COMMAND_NAMES: &[&str] = &[
    "user-prompt-submit",
    "pre-tool-use",
    "post-tool-use",
    "permission-request",
    "stop",
];

const CLIENT_HOOK_COMMAND_NAMES: &[&str] = &[
    "user-prompt-submit",
    "pre-tool-use",
    "post-tool-use",
    "permission-request",
    "stop",
];

const COPILOT_HOOK_EVENTS: &[(&str, &str)] = &[
    ("UserPromptSubmit", "user-prompt-submit"),
    ("PreToolUse", "pre-tool-use"),
    ("PostToolUse", "post-tool-use"),
    ("PermissionRequest", "permission-request"),
    ("Stop", "stop"),
    ("SessionStart", "post-tool-use"),
];

const ANTIGRAVITY_HOOK_EVENTS: &[(&str, &str)] = &[
    ("PreInvocation", "user-prompt-submit"),
    ("PreToolUse", "pre-tool-use"),
    ("PostToolUse", "post-tool-use"),
    ("Stop", "stop"),
];

fn codex_hooks_path(root: &Path) -> PathBuf {
    root.join(".codex/hooks.json")
}

fn claude_hooks_path(root: &Path) -> PathBuf {
    root.join(".claude/settings.local.json")
}

fn codex_hook_dir(root: &Path) -> PathBuf {
    callsieve_dir(root).join("codex-hooks")
}

fn claude_hook_dir(root: &Path) -> PathBuf {
    callsieve_dir(root).join("claude-hooks")
}

fn codex_hook_state_path(root: &Path, session_id: &str) -> PathBuf {
    codex_hook_dir(root).join(format!("{}.state.json", safe_pilot_label(session_id)))
}

fn claude_hook_state_path(root: &Path, session_id: &str) -> PathBuf {
    claude_hook_dir(root).join(format!("{}.state.json", safe_pilot_label(session_id)))
}

fn codex_hook_trace_path(root: &Path, session_id: &str) -> PathBuf {
    codex_hook_dir(root).join(format!("{}.trace.json", safe_pilot_label(session_id)))
}

fn claude_hook_trace_path(root: &Path, session_id: &str) -> PathBuf {
    claude_hook_dir(root).join(format!("{}.trace.json", safe_pilot_label(session_id)))
}

fn client_hooks_path(root: &Path, client: HookClient) -> PathBuf {
    match client {
        HookClient::Copilot => root.join(".github/hooks/callsieve.json"),
        HookClient::OpenCode => root.join(".opencode/plugins/callsieve.js"),
        HookClient::Antigravity => root.join(".agents/hooks.json"),
        HookClient::Cline => root.join(".cline/hooks/callsieve.json"),
    }
}

fn client_hook_dir(root: &Path, client: HookClient) -> PathBuf {
    callsieve_dir(root).join(format!("{}-hooks", hook_client_name(client)))
}

fn client_hook_state_path(root: &Path, client: HookClient, session_id: &str) -> PathBuf {
    client_hook_dir(root, client).join(format!("{}.state.json", safe_pilot_label(session_id)))
}

fn client_hook_trace_path(root: &Path, client: HookClient, session_id: &str) -> PathBuf {
    client_hook_dir(root, client).join(format!("{}.trace.json", safe_pilot_label(session_id)))
}

fn cline_hook_script_path(root: &Path, hook_name: &str, windows: bool) -> PathBuf {
    let extension = if windows { "ps1" } else { "sh" };
    root.join(format!(".cline/hooks/callsieve-{hook_name}.{extension}"))
}

fn hook_client_name(client: HookClient) -> &'static str {
    match client {
        HookClient::Copilot => "copilot",
        HookClient::OpenCode => "opencode",
        HookClient::Antigravity => "antigravity",
        HookClient::Cline => "cline",
    }
}

fn hook_client_display(client: HookClient) -> &'static str {
    match client {
        HookClient::Copilot => "GitHub Copilot",
        HookClient::OpenCode => "OpenCode",
        HookClient::Antigravity => "Antigravity CLI",
        HookClient::Cline => "Cline",
    }
}

fn hook_client_command_prefix(client: HookClient) -> &'static str {
    match client {
        HookClient::Copilot => "copilot-hook",
        HookClient::OpenCode => "opencode-hook",
        HookClient::Antigravity => "antigravity-hook",
        HookClient::Cline => "cline-hook",
    }
}

fn hook_client_agent(client: HookClient) -> AgentClient {
    match client {
        HookClient::Copilot => AgentClient::Copilot,
        HookClient::OpenCode => AgentClient::OpenCode,
        HookClient::Antigravity => AgentClient::Antigravity,
        HookClient::Cline => AgentClient::Cline,
    }
}

fn hook_client_for_agent(client: AgentClient) -> Option<HookClient> {
    match client {
        AgentClient::Copilot => Some(HookClient::Copilot),
        AgentClient::OpenCode => Some(HookClient::OpenCode),
        AgentClient::Antigravity => Some(HookClient::Antigravity),
        AgentClient::Cline => Some(HookClient::Cline),
        _ => None,
    }
}

fn hook_client_policy(client: HookClient) -> &'static str {
    match client {
        HookClient::Copilot => {
            "GitHub Copilot local hooks inject CallSieve context plus local expansion commands and block broad search before context. Cloud agents are template-only unless CallSieve is installed in the sandbox."
        }
        HookClient::OpenCode => {
            "OpenCode plugin hooks inject CallSieve context plus local expansion commands and block broad grep, glob, read, and shell search before context."
        }
        HookClient::Antigravity => {
            "Antigravity CLI hooks inject CallSieve context plus local expansion commands and block broad search before context."
        }
        HookClient::Cline => {
            "Cline lifecycle hooks inject CallSieve context plus local expansion commands and block broad search before context."
        }
    }
}

fn hook_client_trust_instruction(client: HookClient) -> &'static str {
    match client {
        HookClient::Copilot => {
            "Review and trust local GitHub Copilot project hooks before relying on enforcement."
        }
        HookClient::OpenCode => {
            "Review the generated OpenCode plugin before relying on enforcement."
        }
        HookClient::Antigravity => {
            "Review and trust local Antigravity project hooks before relying on enforcement."
        }
        HookClient::Cline => {
            "Review and trust local Cline hook scripts before relying on enforcement."
        }
    }
}

fn hook_client_trace_source(client: HookClient) -> &'static str {
    match client {
        HookClient::Copilot => "github_copilot_lifecycle_hooks",
        HookClient::OpenCode => "opencode_plugin_hooks",
        HookClient::Antigravity => "antigravity_cli_lifecycle_hooks",
        HookClient::Cline => "cline_lifecycle_hooks",
    }
}

fn client_hook_events(client: HookClient) -> &'static [(&'static str, &'static str)] {
    match client {
        HookClient::Copilot => COPILOT_HOOK_EVENTS,
        HookClient::Antigravity => ANTIGRAVITY_HOOK_EVENTS,
        HookClient::OpenCode | HookClient::Cline => &[],
    }
}

fn codex_hooks_json(
    root: &Path,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
) -> serde_json::Value {
    serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [
                codex_hook_entry(codex_hook_command_config(root, "user-prompt-submit", strict, Some((limit, snippets_per_file))))
            ],
            "PreToolUse": [
                codex_hook_entry(codex_hook_command_config(root, "pre-tool-use", strict, None))
            ],
            "PermissionRequest": [
                codex_hook_entry(codex_hook_command_config(root, "permission-request", strict, None))
            ]
        }
    })
}

fn claude_hooks_json(
    root: &Path,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
    force: bool,
) -> Result<serde_json::Value> {
    let hooks_path = claude_hooks_path(root);
    let mut value = if hooks_path.is_file() {
        let content = fs::read_to_string(&hooks_path)
            .with_context(|| format!("failed to read {}", hooks_path.display()))?;
        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(value) if value.is_object() => value,
            Ok(_) if force => serde_json::json!({}),
            Ok(_) => anyhow::bail!("{} must contain a JSON object", hooks_path.display()),
            Err(error) if force => {
                tracing::warn!(
                    path = %hooks_path.display(),
                    error = %error,
                    "replacing invalid Claude Code local settings because --force was used"
                );
                serde_json::json!({})
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to parse {}", hooks_path.display()));
            }
        }
    } else {
        serde_json::json!({})
    };

    remove_claude_hook_entries(&mut value);
    upsert_claude_hook_entry(
        &mut value,
        "UserPromptSubmit",
        None,
        claude_hook_command_config(
            root,
            "user-prompt-submit",
            strict,
            Some((limit, snippets_per_file)),
        ),
    )?;
    for (event, hook_name, matcher) in [
        ("PreToolUse", "pre-tool-use", "Bash|Read|Grep|Glob"),
        // PostToolUse also watches edit tools: confirmed-read learning and
        // edit-impact packets depend on seeing Edit/Write events.
        (
            "PostToolUse",
            "post-tool-use",
            "Bash|Read|Grep|Glob|Edit|Write|MultiEdit|NotebookEdit",
        ),
        (
            "PermissionRequest",
            "permission-request",
            "Bash|Read|Grep|Glob",
        ),
    ] {
        upsert_claude_hook_entry(
            &mut value,
            event,
            Some(matcher),
            claude_hook_command_config(root, hook_name, strict, None),
        )?;
    }
    upsert_claude_hook_entry(
        &mut value,
        "Stop",
        None,
        claude_hook_command_config(root, "stop", strict, None),
    )?;
    Ok(value)
}

fn client_hooks_json(
    root: &Path,
    client: HookClient,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
) -> serde_json::Value {
    let mut hooks = serde_json::Map::new();
    for (event, hook_name) in client_hook_events(client) {
        hooks.insert(
            (*event).to_string(),
            serde_json::json!([client_hook_entry(client_hook_command_config(
                root,
                client,
                hook_name,
                strict,
                context_options(hook_name, limit, snippets_per_file),
            ))]),
        );
    }
    serde_json::json!({
        "version": 1,
        "client": hook_client_name(client),
        "hooks": hooks
    })
}

fn opencode_callsieve_plugin(
    root: &Path,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
) -> String {
    let command = serde_json::to_string(&callsieve_executable_display()).unwrap_or_default();
    let root = serde_json::to_string(&root.display().to_string()).unwrap_or_default();
    let strict = if strict { "true" } else { "false" };
    format!(
        "const {{ spawnSync }} = require('node:child_process');\n\
const callsieveCommand = {command};\n\
const repoRoot = {root};\n\
const strictMode = {strict};\n\
\n\
function argsFor(hookName) {{\n\
  const args = ['opencode-hook', hookName, repoRoot];\n\
  if (strictMode) args.push('--strict');\n\
  if (hookName === 'user-prompt-submit') args.push('--limit', '{limit}', '--snippets-per-file', '{snippets_per_file}');\n\
  return args;\n\
}}\n\
\n\
function runCallsieve(hookName, payload) {{\n\
  const result = spawnSync(callsieveCommand, argsFor(hookName), {{ input: JSON.stringify(payload || {{}}), encoding: 'utf8' }});\n\
  if (result.status !== 0) return {{ hookSpecificOutput: {{ permissionDecision: 'deny', permissionDecisionReason: result.stderr || 'CallSieve hook failed' }} }};\n\
  try {{ return JSON.parse(result.stdout || '{{}}'); }} catch (_) {{ return {{}}; }}\n\
}}\n\
\n\
module.exports = async function callsievePlugin() {{\n\
  return {{\n\
    name: 'callsieve',\n\
    'session.start': async (session) => runCallsieve('user-prompt-submit', session),\n\
    'tool.execute.before': async (tool) => {{\n\
      const output = runCallsieve('pre-tool-use', tool);\n\
      const hook = output.hookSpecificOutput || {{}};\n\
      if (hook.permissionDecision === 'deny') return {{ error: hook.permissionDecisionReason || 'CallSieve denied pre-context tool use' }};\n\
      return tool;\n\
    }},\n\
    'tool.execute.after': async (tool) => runCallsieve('post-tool-use', tool),\n\
    'session.end': async (session) => runCallsieve('stop', session),\n\
  }};\n\
}};\n"
    )
}

fn cline_hooks_manifest(
    root: &Path,
    strict: bool,
    limit: usize,
    snippets_per_file: usize,
) -> String {
    let mut hooks = serde_json::Map::new();
    for hook_name in CLIENT_HOOK_COMMAND_NAMES {
        hooks.insert(
            (*hook_name).to_string(),
            serde_json::json!({
                "windows": repo_relative_display(root, &cline_hook_script_path(root, hook_name, true)),
                "unix": repo_relative_display(root, &cline_hook_script_path(root, hook_name, false)),
                "strict": strict,
                "limit": if *hook_name == "user-prompt-submit" { Some(limit) } else { None },
                "snippets_per_file": if *hook_name == "user-prompt-submit" { Some(snippets_per_file) } else { None }
            }),
        );
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "client": "cline",
        "hooks": hooks
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn cline_hook_script_ps1(
    root: &Path,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
) -> String {
    format!(
        "$inputText = [Console]::In.ReadToEnd()\n\
if ([string]::IsNullOrWhiteSpace($inputText)) {{ $inputText = '{{}}' }}\n\
$inputText | {}\n",
        client_hook_command_line(
            root,
            HookClient::Cline,
            hook_name,
            strict,
            context_options,
            true
        )
    )
}

fn cline_hook_script_sh(
    root: &Path,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
) -> String {
    format!(
        "#!/usr/bin/env sh\nexec {}\n",
        client_hook_command_line(
            root,
            HookClient::Cline,
            hook_name,
            strict,
            context_options,
            false
        )
    )
}

fn context_options(
    hook_name: &str,
    limit: usize,
    snippets_per_file: usize,
) -> Option<(usize, usize)> {
    (hook_name == "user-prompt-submit").then_some((limit, snippets_per_file))
}

fn codex_hook_entry(command: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "hooks": [command]
    })
}

fn client_hook_entry(command: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "hooks": [command]
    })
}

fn claude_hook_entry(matcher: Option<&str>, command: serde_json::Value) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "hooks": [command]
    });
    if let Some(matcher) = matcher {
        entry
            .as_object_mut()
            .expect("Claude hook entry must be an object")
            .insert("matcher".to_string(), serde_json::json!(matcher));
    }
    entry
}

fn upsert_claude_hook_entry(
    value: &mut serde_json::Value,
    event: &str,
    matcher: Option<&str>,
    command: serde_json::Value,
) -> Result<()> {
    let object = value
        .as_object_mut()
        .context("Claude Code settings root must be a JSON object")?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    let hooks = hooks
        .as_object_mut()
        .context("Claude Code hooks root must be a JSON object")?;
    let entries = hooks.entry(event).or_insert_with(|| serde_json::json!([]));
    if !entries.is_array() {
        *entries = serde_json::json!([]);
    }
    entries
        .as_array_mut()
        .context("Claude Code hook event must be an array")?
        .push(claude_hook_entry(matcher, command));
    Ok(())
}

fn remove_claude_hook_entries(value: &mut serde_json::Value) {
    let Some(hooks) = value
        .get_mut("hooks")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };

    for event in CLAUDE_HOOK_EVENTS {
        let Some(entries) = hooks
            .get_mut(*event)
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        entries.retain_mut(|entry| {
            let Some(handlers) = entry
                .get_mut("hooks")
                .and_then(serde_json::Value::as_array_mut)
            else {
                return true;
            };
            handlers.retain(|handler| !is_claude_callsieve_handler(handler));
            !handlers.is_empty()
        });
    }
    hooks.retain(|_, event_hooks| {
        event_hooks
            .as_array()
            .is_none_or(|entries| !entries.is_empty())
    });
}

fn is_claude_callsieve_handler(handler: &serde_json::Value) -> bool {
    let command_is_callsieve = handler
        .get("command")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|command| command.contains("callsieve"));
    let args_include_hook = handler
        .get("args")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|args| {
            args.iter()
                .filter_map(serde_json::Value::as_str)
                .any(|arg| arg == "claude-hook")
        });
    command_is_callsieve && args_include_hook
}

fn codex_hook_command_config(
    root: &Path,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": codex_hook_command_line(root, hook_name, strict, context_options, false),
        "commandWindows": codex_hook_command_line(root, hook_name, strict, context_options, true)
    })
}

fn claude_hook_command_config(
    root: &Path,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": callsieve_executable_display(),
        "args": claude_hook_command_args(root, hook_name, strict, context_options),
        "timeout": if hook_name == "user-prompt-submit" { 30 } else { 60 },
        "statusMessage": "CallSieve context policy"
    })
}

fn claude_hook_command_args(
    root: &Path,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
) -> Vec<String> {
    let mut args = vec![
        "claude-hook".to_string(),
        hook_name.to_string(),
        root.display().to_string(),
    ];
    if strict {
        args.push("--strict".to_string());
    }
    if let Some((limit, snippets_per_file)) = context_options {
        args.push("--limit".to_string());
        args.push(limit.to_string());
        args.push("--snippets-per-file".to_string());
        args.push(snippets_per_file.to_string());
    }
    args
}

fn client_hook_command_config(
    root: &Path,
    client: HookClient,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": client_hook_command_line(root, client, hook_name, strict, context_options, false),
        "commandWindows": client_hook_command_line(root, client, hook_name, strict, context_options, true)
    })
}

fn client_hook_command_args(
    root: &Path,
    client: HookClient,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
) -> Vec<String> {
    let mut args = vec![
        hook_client_command_prefix(client).to_string(),
        hook_name.to_string(),
        root.display().to_string(),
    ];
    if strict {
        args.push("--strict".to_string());
    }
    if let Some((limit, snippets_per_file)) = context_options {
        args.push("--limit".to_string());
        args.push(limit.to_string());
        args.push("--snippets-per-file".to_string());
        args.push(snippets_per_file.to_string());
    }
    args
}

fn client_hook_command_line(
    root: &Path,
    client: HookClient,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
    windows: bool,
) -> String {
    let args = client_hook_command_args(root, client, hook_name, strict, context_options);
    let executable = callsieve_executable_display();
    if windows {
        let mut parts = vec![windows_cmd_arg(&executable)];
        parts.extend(args.iter().map(|arg| windows_cmd_arg(arg)));
        parts.join(" ")
    } else {
        let mut parts = vec![sh_cmd_arg(&executable)];
        parts.extend(args.iter().map(|arg| sh_cmd_arg(arg)));
        parts.join(" ")
    }
}

fn codex_hook_command_line(
    root: &Path,
    hook_name: &str,
    strict: bool,
    context_options: Option<(usize, usize)>,
    windows: bool,
) -> String {
    let mut args = vec![
        "codex-hook".to_string(),
        hook_name.to_string(),
        root.display().to_string(),
    ];
    if strict {
        args.push("--strict".to_string());
    }
    if let Some((limit, snippets_per_file)) = context_options {
        args.push("--limit".to_string());
        args.push(limit.to_string());
        args.push("--snippets-per-file".to_string());
        args.push(snippets_per_file.to_string());
    }

    let executable = callsieve_executable_display();
    if windows {
        let mut parts = vec![windows_cmd_arg(&executable)];
        parts.extend(args.iter().map(|arg| windows_cmd_arg(arg)));
        parts.join(" ")
    } else {
        let mut parts = vec![sh_cmd_arg(&executable)];
        parts.extend(args.iter().map(|arg| sh_cmd_arg(arg)));
        parts.join(" ")
    }
}

fn windows_cmd_arg(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn sh_cmd_arg(value: &str) -> String {
    format!("'{}'", sh_single_quote(value))
}

fn read_hook_input(client: &str) -> Result<serde_json::Value> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    if input.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_str(&input)
        .with_context(|| format!("failed to parse {client} hook input JSON"))
}

fn read_codex_hook_input() -> Result<serde_json::Value> {
    read_hook_input("Codex")
}

fn read_claude_hook_input() -> Result<serde_json::Value> {
    read_hook_input("Claude Code")
}

fn hook_session_id(input: &serde_json::Value) -> String {
    hook_string_field(input, &["session_id", "sessionId"])
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "local-hook-session".to_string())
}

fn hook_turn_id(input: &serde_json::Value) -> String {
    hook_string_field(input, &["turn_id", "turnId"])
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("turn-{}", now_unix_seconds()))
}

fn hook_tool_name(input: &serde_json::Value) -> String {
    hook_string_field(input, &["tool_name", "toolName", "tool"])
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Repo-relativize a tool-reported path against a possibly relative or
/// symlinked hook root: canonicalize both sides before stripping, so
/// `.`-installed hooks and macOS /tmp symlinks still match index paths.
fn hook_repo_relative(path: &str, root: &Path) -> String {
    let candidate = Path::new(path);
    if candidate.is_relative() {
        return path.to_string();
    }
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canonical = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    canonical
        .strip_prefix(&canonical_root)
        .map(|relative| relative.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
}

/// File path from an edit/write tool event, repo-relative when possible.
fn hook_edited_file(input: &serde_json::Value, root: &Path) -> Option<String> {
    let tool_name = hook_tool_name(input).to_ascii_lowercase();
    let is_edit = matches!(
        tool_name.as_str(),
        "edit" | "write" | "multiedit" | "str_replace" | "str_replace_editor" | "notebookedit"
    ) || tool_name.ends_with("__edit")
        || tool_name.ends_with("__write");
    if !is_edit {
        return None;
    }
    let tool_input = input
        .get("tool_input")
        .or_else(|| input.get("toolInput"))
        .or_else(|| input.get("input"))?;
    let path = hook_json_string_field(
        tool_input,
        &["file_path", "filePath", "path", "notebook_path"],
    )?;
    Some(hook_repo_relative(&path, root))
}

fn hook_tool_command(input: &serde_json::Value) -> String {
    let tool_name = hook_tool_name(input);
    let tool_input = input
        .get("tool_input")
        .or_else(|| input.get("toolInput"))
        .or_else(|| input.get("input"));
    if let Some(command) = tool_input
        .and_then(|value| value.get("command"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            tool_input
                .and_then(|value| value.get("cmd"))
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| input.get("command").and_then(serde_json::Value::as_str))
    {
        return command.to_string();
    }
    if let Some(tool_input) = tool_input {
        let lower_tool = tool_name.to_ascii_lowercase();
        if (matches!(lower_tool.as_str(), "read" | "read_file" | "view_file")
            || lower_tool.ends_with("__read"))
            && let Some(path) =
                hook_json_string_field(tool_input, &["file_path", "filePath", "path"])
        {
            return format!("{tool_name} {path}");
        }
        if matches!(
            lower_tool.as_str(),
            "grep" | "grep_search" | "find_by_name" | "codebase_search"
        ) || lower_tool.ends_with("__grep")
        {
            let mut parts = Vec::new();
            for field in ["pattern", "path", "glob"] {
                if let Some(value) = hook_json_string_field(tool_input, &[field]) {
                    parts.push(value);
                }
            }
            if !parts.is_empty() {
                return format!("{tool_name} {}", parts.join(" "));
            }
        }
        if lower_tool == "glob" || lower_tool.ends_with("__glob") {
            let mut parts = Vec::new();
            for field in ["pattern", "path"] {
                if let Some(value) = hook_json_string_field(tool_input, &[field]) {
                    parts.push(value);
                }
            }
            if !parts.is_empty() {
                return format!("{tool_name} {}", parts.join(" "));
            }
        }
    }
    if tool_name.to_ascii_lowercase().contains("callsieve_context") {
        return "callsieve_context".to_string();
    }
    if let Some(value) = tool_input.and_then(serde_json::Value::as_str) {
        return format!("{tool_name} {value}");
    }
    if let Some(value) = tool_input {
        return format!("{tool_name} {value}");
    }
    tool_name
}

fn hook_string_field(input: &serde_json::Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .find_map(|field| input.get(*field).and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

fn hook_json_string_field(input: &serde_json::Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .find_map(|field| input.get(*field).and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

fn load_codex_hook_state(root: &Path, session_id: &str) -> CodexHookState {
    fs::read(codex_hook_state_path(root, session_id))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_else(|| CodexHookState {
            version: 1,
            session_id: session_id.to_string(),
            root: root_label(root),
            ..CodexHookState::default()
        })
}

fn save_codex_hook_state(root: &Path, state: &CodexHookState) -> Result<()> {
    let dir = codex_hook_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(
        codex_hook_state_path(root, &state.session_id),
        serde_json::to_vec_pretty(state)?,
    )
    .with_context(|| format!("failed to write Codex hook state for {}", root.display()))
}

fn load_claude_hook_state(root: &Path, session_id: &str) -> CodexHookState {
    fs::read(claude_hook_state_path(root, session_id))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_else(|| CodexHookState {
            version: 1,
            session_id: session_id.to_string(),
            root: root_label(root),
            ..CodexHookState::default()
        })
}

fn save_claude_hook_state(root: &Path, state: &CodexHookState) -> Result<()> {
    let dir = claude_hook_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(
        claude_hook_state_path(root, &state.session_id),
        serde_json::to_vec_pretty(state)?,
    )
    .with_context(|| {
        format!(
            "failed to write Claude Code hook state for {}",
            root.display()
        )
    })
}

fn load_client_hook_state(root: &Path, client: HookClient, session_id: &str) -> CodexHookState {
    fs::read(client_hook_state_path(root, client, session_id))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_else(|| CodexHookState {
            version: 1,
            session_id: session_id.to_string(),
            root: root_label(root),
            ..CodexHookState::default()
        })
}

fn save_client_hook_state(root: &Path, client: HookClient, state: &CodexHookState) -> Result<()> {
    let dir = client_hook_dir(root, client);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(
        client_hook_state_path(root, client, &state.session_id),
        serde_json::to_vec_pretty(state)?,
    )
    .with_context(|| {
        format!(
            "failed to write {} hook state for {}",
            hook_client_display(client),
            root.display()
        )
    })
}

fn append_codex_hook_trace_event(
    root: &Path,
    state: &CodexHookState,
    task: &str,
    event: serde_json::Value,
) -> Result<String> {
    let path = codex_hook_trace_path(root, &state.session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut value = if path.is_file() {
        read_trace_value(&path)?
    } else {
        codex_hook_trace_template(root, state, task)
    };
    value
        .as_object_mut()
        .context("Codex hook trace root must be a JSON object")?
        .insert("task".to_string(), serde_json::json!(task));
    let events = value
        .as_object_mut()
        .context("Codex hook trace root must be a JSON object")?
        .entry("events")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Codex hook trace events must be an array")?;
    events.push(event);
    normalize_session_trace(&mut value)?;
    fs::write(&path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path.display().to_string())
}

fn append_claude_hook_trace_event(
    root: &Path,
    state: &CodexHookState,
    task: &str,
    event: serde_json::Value,
) -> Result<String> {
    let path = claude_hook_trace_path(root, &state.session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut value = if path.is_file() {
        read_trace_value(&path)?
    } else {
        claude_hook_trace_template(root, state, task)
    };
    value
        .as_object_mut()
        .context("Claude Code hook trace root must be a JSON object")?
        .insert("task".to_string(), serde_json::json!(task));
    let events = value
        .as_object_mut()
        .context("Claude Code hook trace root must be a JSON object")?
        .entry("events")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("Claude Code hook trace events must be an array")?;
    events.push(event);
    normalize_session_trace(&mut value)?;
    fs::write(&path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path.display().to_string())
}

fn append_client_hook_trace_event(
    root: &Path,
    client: HookClient,
    state: &CodexHookState,
    task: &str,
    event: serde_json::Value,
) -> Result<String> {
    let path = client_hook_trace_path(root, client, &state.session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut value = if path.is_file() {
        read_trace_value(&path)?
    } else {
        client_hook_trace_template(root, client, state, task)
    };
    value
        .as_object_mut()
        .context("client hook trace root must be a JSON object")?
        .insert("task".to_string(), serde_json::json!(task));
    let events = value
        .as_object_mut()
        .context("client hook trace root must be a JSON object")?
        .entry("events")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("client hook trace events must be an array")?;
    events.push(event);
    normalize_session_trace(&mut value)?;
    fs::write(&path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path.display().to_string())
}

fn codex_hook_trace_template(root: &Path, state: &CodexHookState, task: &str) -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "collection": "codex_hook_trace",
            "client": "codex",
            "repo": root_label(root),
            "session_id": state.session_id.clone(),
            "strict": state.strict,
            "started_at": now_unix_seconds(),
            "updated_at": now_unix_seconds()
        },
        "task": task,
        "expected_files": [],
        "critical_files": [],
        "baseline": empty_session_metrics(),
        "callsieve": empty_session_metrics(),
        "session": {
            "baseline": empty_session_metrics(),
            "callsieve": empty_session_metrics()
        },
        "events": [],
        "misses": [],
        "policy": {
            "context_first": true,
            "strict_trace_check": state.strict,
            "source": "codex_lifecycle_hooks"
        }
    })
}

fn claude_hook_trace_template(
    root: &Path,
    state: &CodexHookState,
    task: &str,
) -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "collection": "claude_hook_trace",
            "client": "claude",
            "repo": root_label(root),
            "session_id": state.session_id.clone(),
            "strict": state.strict,
            "started_at": now_unix_seconds(),
            "updated_at": now_unix_seconds()
        },
        "task": task,
        "expected_files": [],
        "critical_files": [],
        "baseline": empty_session_metrics(),
        "callsieve": empty_session_metrics(),
        "session": {
            "baseline": empty_session_metrics(),
            "callsieve": empty_session_metrics()
        },
        "events": [],
        "misses": [],
        "policy": {
            "context_first": true,
            "strict_trace_check": state.strict,
            "source": "claude_code_lifecycle_hooks"
        }
    })
}

fn client_hook_trace_template(
    root: &Path,
    client: HookClient,
    state: &CodexHookState,
    task: &str,
) -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "collection": format!("{}_hook_trace", hook_client_name(client)),
            "client": hook_client_name(client),
            "repo": root_label(root),
            "session_id": state.session_id.clone(),
            "strict": state.strict,
            "started_at": now_unix_seconds(),
            "updated_at": now_unix_seconds()
        },
        "task": task,
        "expected_files": [],
        "critical_files": [],
        "baseline": empty_session_metrics(),
        "callsieve": empty_session_metrics(),
        "session": {
            "baseline": empty_session_metrics(),
            "callsieve": empty_session_metrics()
        },
        "events": [],
        "misses": [],
        "policy": {
            "context_first": true,
            "strict_trace_check": state.strict,
            "source": hook_client_trace_source(client)
        }
    })
}

fn codex_hook_trace_event(
    input: &serde_json::Value,
    command: &str,
    policy_violation: bool,
) -> serde_json::Value {
    serde_json::json!({
        "timestamp": now_unix_seconds(),
        "command": command,
        "files_read": codex_hook_files_read(command),
        "tokens": 0,
        "classification": classify_session_command(command),
        "phase": "callsieve",
        "hook_event": hook_string_field(input, &["hook_event_name", "hookEventName"]).unwrap_or_default(),
        "tool_name": hook_tool_name(input),
        "policy_violation": policy_violation
    })
}

const MAX_CONFIRMED_READS: usize = 32;

/// Record agent-observed file reads that happened after context was served;
/// these confirm which suggestions (or discoveries) the agent actually used.
fn record_confirmed_reads(state: &mut CodexHookState, root: &Path, command: &str) {
    if !state.context_seen {
        return;
    }
    for read in codex_hook_files_read(command) {
        if state.confirmed_reads.len() >= MAX_CONFIRMED_READS {
            break;
        }
        let relative = hook_repo_relative(&read, root);
        // Bash read commands carry flags, patterns, and pipe segments as
        // tokens; only paths that actually exist under the repo are evidence.
        if !root.join(&relative).is_file() {
            continue;
        }
        if !state.confirmed_reads.contains(&relative) {
            state.confirmed_reads.push(relative);
        }
    }
}

fn codex_hook_files_read(command: &str) -> Vec<String> {
    if !is_file_read_command_local(command) {
        return Vec::new();
    }
    command
        .split_whitespace()
        .filter(|part| {
            let lower = part.to_ascii_lowercase();
            !matches!(
                lower.as_str(),
                "cat"
                    | "less"
                    | "more"
                    | "head"
                    | "tail"
                    | "sed"
                    | "nl"
                    | "bat"
                    | "type"
                    | "read"
                    | "get-content"
                    | "gc"
            ) && !lower.starts_with('-')
        })
        .map(|part| part.trim_matches('"').trim_matches('\'').to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

fn codex_hook_should_deny(state: &CodexHookState, strict: bool, command: &str) -> bool {
    if state.context_seen || command.trim().is_empty() {
        return false;
    }
    if codex_hook_command_allowed_without_context(command) {
        return false;
    }
    is_broad_search_command_local(command)
        || (strict
            && is_file_read_command_local(command)
            && !codex_hook_allowed_pre_context_read(command))
}

fn codex_hook_command_allowed_without_context(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    first == "callsieve"
        || lower.starts_with("git status")
        || first == "cargo"
        || first == "rust-analyzer"
        || first == "pwd"
        || first == "date"
        || first == "echo"
}

fn codex_hook_allowed_pre_context_read(command: &str) -> bool {
    let normalized = command.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("agents.md")
        || normalized.contains("claude.md")
        || normalized.contains(".mcp.json")
        || normalized.contains(".codex/callsieve.md")
        || normalized.contains(".codex/hooks.json")
        || normalized.contains(".codex/config.toml")
        || normalized.contains(".claude/settings.local.json")
        || normalized.contains(".claude/settings.json")
        || normalized.contains(".github/copilot-instructions.md")
        || normalized.contains(".github/agents/callsieve-context.agent.md")
        || normalized.contains(".github/copilot-mcp.json")
        || normalized.contains(".github/hooks/callsieve.json")
        || normalized.contains("opencode.json")
        || normalized.contains(".opencode/callsieve.md")
        || normalized.contains(".opencode/plugins/callsieve.js")
        || normalized.contains(".agents/mcp_config.json")
        || normalized.contains(".agents/hooks.json")
        || normalized.contains(".agents/skills/callsieve-context.md")
        || normalized.contains(".agents/rules/callsieve.md")
        || normalized.contains(".cursor/mcp.json")
        || normalized.contains(".cursor/rules/callsieve.mdc")
        || normalized.contains(".cline/mcp.json")
        || normalized.contains(".cline/hooks/")
        || normalized.contains(".cline/rules/callsieve.md")
        || normalized.contains(".clinerules/callsieve.md")
        || normalized.contains(".roo/mcp.json")
        || normalized.contains(".roo/rules/callsieve.md")
        || normalized.contains(".roo/rules-code/callsieve.md")
        || normalized.contains(".roomodes")
}

fn codex_hook_permission_response(event: &str, decision: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "permissionDecision": decision,
            "permissionDecisionReason": reason
        }
    })
}

fn codex_hook_noop_response(event: &str) -> serde_json::Value {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": event
        }
    })
}

fn claude_hook_pre_tool_response(event: &str, decision: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": event,
            "permissionDecision": decision,
            "permissionDecisionReason": reason
        }
    })
}

fn client_hook_permission_response(event: &str, decision: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "suppressOutput": true,
        "hookSpecificOutput": {
            "hookEventName": event,
            "permissionDecision": decision,
            "permissionDecisionReason": reason
        }
    })
}

fn codex_hook_denial_reason(command: &str, strict: bool) -> String {
    if strict && is_file_read_command_local(command) && !is_broad_search_command_local(command) {
        "CallSieve needs context before file reads in strict mode. Read the context packet first, or run callsieve agent-context, then retry if needed.".to_string()
    } else {
        "CallSieve needs context before broad repo search. Read the context packet first, or run callsieve agent-context, then retry if needed.".to_string()
    }
}

fn hook_hash(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn write_hook_launchers(
    root: &Path,
    first_required_command: &str,
    force: bool,
) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for path in hook_launcher_paths(root) {
        if path.exists() && !force {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let content = if path.extension().and_then(|extension| extension.to_str()) == Some("ps1") {
            hook_launcher_ps1(root, first_required_command)
        } else {
            hook_launcher_sh(root, first_required_command)
        };
        write_executable_file(&path, &content)?;
        files.push(repo_relative_display(root, &path));
    }
    Ok(files)
}

fn hook_launcher_paths(root: &Path) -> [PathBuf; 2] {
    [
        root.join(".callsieve/agent-launch.ps1"),
        root.join(".callsieve/agent-launch.sh"),
    ]
}

fn hook_launcher_ps1(root: &Path, first_required_command: &str) -> String {
    let root = root.display().to_string().replace('\'', "''");
    let first_required_command = first_required_command.replace('\'', "''");
    format!(
        "$ErrorActionPreference = 'Stop'\n\
$repo = '{root}'\n\
$env:PATH = \"$repo\\.callsieve\\bin;$env:PATH\"\n\
callsieve daemon $repo --background --lsp | Out-Null\n\
Write-Host 'CallSieve hook active for this process only.'\n\
Write-Host 'First task command: {first_required_command}'\n\
if ($args.Count -gt 0) {{\n\
  $cmd = $args[0]\n\
  $rest = @()\n\
  if ($args.Count -gt 1) {{ $rest = $args[1..($args.Count - 1)] }}\n\
  & $cmd @rest\n\
}}\n"
    )
}

fn hook_launcher_sh(root: &Path, first_required_command: &str) -> String {
    let root = sh_single_quote(&root.display().to_string());
    let first_required_command = sh_single_quote(first_required_command);
    format!(
        "#!/usr/bin/env sh\n\
REPO='{root}'\n\
export PATH=\"$REPO/.callsieve/bin:$PATH\"\n\
callsieve daemon \"$REPO\" --background --lsp >/dev/null 2>&1 || true\n\
printf '%s\\n' 'CallSieve hook active for this process only.'\n\
printf '%s\\n' 'First task command: {first_required_command}'\n\
if [ \"$#\" -gt 0 ]; then exec \"$@\"; fi\n"
    )
}

fn write_codex_launchers(
    root: &Path,
    first_required_command: &str,
    force: bool,
) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for path in codex_launcher_paths(root) {
        if path.exists() && !force {
            continue;
        }
        let content = if path.extension().and_then(|extension| extension.to_str()) == Some("ps1") {
            codex_launcher_ps1(first_required_command)
        } else {
            codex_launcher_sh(first_required_command)
        };
        write_project_file(root, &path, &content, force, &mut files)?;
    }
    Ok(files)
}

fn codex_launcher_paths(root: &Path) -> [PathBuf; 2] {
    [
        root.join(".callsieve/codex-launch.ps1"),
        root.join(".callsieve/codex-launch.sh"),
    ]
}

fn editor_hook(root: &Path, editor: EditorKind, force: bool) -> Result<EditorHookOutput> {
    let mut files = Vec::new();
    let daemon_command = format!("callsieve daemon {} --background --lsp", root.display());

    match editor {
        EditorKind::Vscode => {
            write_project_file(
                root,
                &root.join(".vscode/tasks.json"),
                &vscode_task_json(),
                force,
                &mut files,
            )?;
            write_project_file(
                root,
                &root.join(".codex/CALLSIEVE.md"),
                codex_policy_text(),
                force,
                &mut files,
            )?;
        }
        EditorKind::Cursor => {
            let setup = setup_agent(AgentClient::Cursor, root, force)?;
            files.extend(setup.files);
            write_project_file(
                root,
                &root.join(".cursor/tasks.json"),
                &cursor_task_json(),
                force,
                &mut files,
            )?;
        }
        EditorKind::Generic => {
            write_project_file(
                root,
                &root.join(".callsieve/editor-hook.md"),
                &format!(
                    "Run `{daemon_command}` at project open, then call `callsieve agent-context <repo> \"<task>\"` before broad grep or repeated file reads. Read `read_first`, review `context.sel.next`, then use `instruction.x.o/n/next`, `callsieve focus`, `callsieve related`, and `callsieve tests` before grep.\n"
                ),
                force,
                &mut files,
            )?;
            write_project_file(
                root,
                &root.join(".callsieve/editor-hook.json"),
                &serde_json::to_string_pretty(&serde_json::json!({
                    "daemon_command": daemon_command,
                    "policy": "callsieve_context before broad grep or repeated file reads",
                    "ranked_expansion_before_grep": ["context.sel.next", "instruction.x.o/n/next"],
                    "local_expansion_before_grep": ["callsieve focus", "callsieve related", "callsieve tests"]
                }))?,
                force,
                &mut files,
            )?;
        }
    }

    Ok(EditorHookOutput {
        command: "editor-hook",
        root: root_label(root),
        editor: editor_name(editor).to_string(),
        files,
        daemon_command,
    })
}

fn write_project_file(
    root: &Path,
    path: &Path,
    content: &str,
    force: bool,
    written: &mut Vec<String>,
) -> Result<()> {
    if path.exists() && !force {
        anyhow::bail!(
            "refusing to overwrite {}; pass --force to replace it",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;
    written.push(repo_relative_display(root, path));
    Ok(())
}

fn codex_launcher_ps1(first_required_command: &str) -> String {
    format!(
        "$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path\r\n$env:PATH = (Join-Path $Repo '.callsieve/bin') + ';' + $env:PATH\r\ncallsieve daemon $Repo --background --lsp | Out-Null\r\nWrite-Output 'CallSieve daemon requested for this repo.'\r\nWrite-Output 'First command: {first_required_command}'\r\n"
    )
}

fn codex_launcher_sh(first_required_command: &str) -> String {
    format!(
        "#!/usr/bin/env sh\nrepo=\"$(CDPATH= cd -- \"$(dirname -- \"$0\")/..\" && pwd)\"\nexport PATH=\"$repo/.callsieve/bin:$PATH\"\ncallsieve daemon \"$repo\" --background --lsp >/dev/null 2>&1 || true\nprintf '%s\\n' 'CallSieve daemon requested for this repo.'\nprintf '%s\\n' 'First command: {first_required_command}'\n"
    )
}

fn vscode_task_json() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "version": "2.0.0",
        "tasks": [
            {
                "label": "CallSieve daemon",
                "type": "shell",
                "command": "callsieve daemon \"${workspaceFolder}\" --background --lsp",
                "problemMatcher": []
            }
        ]
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn cursor_task_json() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "tasks": [
            {
                "label": "CallSieve daemon",
                "command": "callsieve daemon . --background --lsp"
            }
        ]
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn codex_policy_text() -> &'static str {
    "CallSieve policy: call callsieve_context with the repository path and task before broad grep, rg, repository-wide search, or repeated file reads. Read read_first files first, review context.sel.next, then use instruction.x.o/n/next plus callsieve_focus, callsieve_related, and callsieve_tests, or CLI callsieve focus, callsieve related, and callsieve tests, before grep. Grep only if the context packet and local expansion are insufficient.\n"
}

fn editor_name(editor: EditorKind) -> &'static str {
    match editor {
        EditorKind::Vscode => "vscode",
        EditorKind::Cursor => "cursor",
        EditorKind::Generic => "generic",
    }
}

fn enforce_setup(
    root: &Path,
    client: AgentClient,
    trace: Option<&Path>,
    strict: bool,
    require_shim: bool,
) -> Result<EnforceOutput> {
    let mut checks = Vec::new();
    let index = store::json_store::load_index(root).ok();
    let status = query::index_status(root, index.as_ref());
    let status_value = serde_json::to_value(&status)?;
    let fresh = status_value
        .get("fresh")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    checks.push(enforce_check(
        "fresh_index",
        fresh,
        if fresh {
            "index is fresh"
        } else {
            "index is missing or stale"
        },
    ));

    for (path, _) in agent_files(client, root) {
        let relative = repo_relative_display(root, &path);
        checks.push(enforce_check(
            format!("agent_file:{relative}"),
            path.is_file(),
            if path.is_file() {
                "required agent policy/config exists".to_string()
            } else {
                "required agent policy/config is missing".to_string()
            },
        ));
    }

    if matches!(client, AgentClient::Codex) && (strict || require_shim) {
        for path in codex_launcher_paths(root) {
            let relative = repo_relative_display(root, &path);
            checks.push(enforce_check(
                format!("codex_bootstrap:{relative}"),
                path.is_file(),
                if path.is_file() {
                    "Codex bootstrap launcher exists".to_string()
                } else {
                    "Codex bootstrap launcher is missing".to_string()
                },
            ));
        }
    }
    if matches!(client, AgentClient::Codex) && strict {
        let codex_hooks = codex_hooks_doctor(root, true);
        checks.push(enforce_check(
            "codex_hooks",
            codex_hooks.status == "pass",
            if codex_hooks.status == "pass" {
                "Codex lifecycle hooks are installed"
            } else {
                "strict Codex enforcement requires lifecycle hooks"
            },
        ));
    }
    if matches!(client, AgentClient::Claude) && strict {
        let claude_hooks = claude_hooks_doctor(root, true);
        checks.push(enforce_check(
            "claude_hooks",
            claude_hooks.status == "pass",
            if claude_hooks.status == "pass" {
                "Claude Code lifecycle hooks are installed"
            } else {
                "strict Claude Code enforcement requires lifecycle hooks"
            },
        ));
    }
    if let Some(hook_client) = hook_client_for_agent(client)
        && strict
    {
        let hooks = client_hooks_doctor(root, hook_client, true);
        checks.push(enforce_check(
            format!("{}_hooks", hook_client_name(hook_client)),
            hooks.status == "pass",
            if hooks.status == "pass" {
                format!("{} hooks are installed", hook_client_display(hook_client))
            } else {
                format!(
                    "strict {} enforcement requires hooks",
                    hook_client_display(hook_client)
                )
            },
        ));
    }

    let shim = shim_doctor(root);
    let shim_files = shim_files_installed(root);
    if strict && !matches!(client, AgentClient::Generic) {
        checks.push(enforce_check(
            "shim_files",
            shim_files,
            if shim_files {
                "strict mode shim files are installed"
            } else {
                "strict mode requires project-local rg/grep shims"
            },
        ));
    }
    checks.push(EnforceCheck {
        check: "shim_doctor".to_string(),
        status: if shim.status == "pass" || (require_shim && shim_files) {
            "pass"
        } else if require_shim {
            "fail"
        } else {
            "warn"
        }
        .to_string(),
        message: if shim.status == "pass" {
            "grep shim is installed and on PATH"
        } else if require_shim && shim_files {
            "grep shim files are installed; prepend shim bin directory to PATH before running agents"
        } else if require_shim {
            "grep shim is required but wrapper files are missing"
        } else {
            "grep shim is optional; use --require-shim to fail on this"
        }
        .to_string(),
    });
    let bin_dir = shim_bin_dir(root);
    let shim_on_path = shim_dir_on_path(&bin_dir);
    checks.push(check_with_status(
        "path_contains_shim_dir",
        if shim_on_path { "pass" } else { "warn" },
        if shim_on_path {
            "shim bin directory is on PATH"
        } else {
            "prepend shim bin directory to PATH before running agents"
        },
    ));

    if let Some(trace) = trace {
        let trace_json = fs::read_to_string(trace)
            .with_context(|| format!("failed to read trace: {}", trace.display()))?;
        let trace_check = if strict {
            query::trace_check_from_str_with_options(&trace_json, true)?
        } else {
            query::trace_check_from_str(&trace_json)?
        };
        let trace_value = serde_json::to_value(trace_check)?;
        let pass = trace_value
            .get("status")
            .and_then(serde_json::Value::as_str)
            == Some("pass");
        checks.push(enforce_check(
            "trace_policy",
            pass,
            if pass {
                "trace obeys context-first policy"
            } else {
                "trace violates context-first policy"
            },
        ));
        if matches!(client, AgentClient::Codex) && strict {
            let trace_value: serde_json::Value = serde_json::from_str(&trace_json)?;
            let is_observed = trace_value
                .get("metadata")
                .and_then(|metadata| metadata.get("collection"))
                .and_then(serde_json::Value::as_str)
                == Some("observed_session");
            checks.push(enforce_check(
                "trace_collection_observed_session",
                is_observed,
                if is_observed {
                    "trace is real observed-session evidence"
                } else {
                    "strict Codex evidence requires metadata.collection = observed_session"
                },
            ));
        }
    }

    let status = if checks.iter().all(|check| check.status != "fail") {
        "pass"
    } else {
        "fail"
    }
    .to_string();

    Ok(EnforceOutput {
        command: "enforce",
        status,
        root: root_label(root),
        client: agent_client_name(client).to_string(),
        checks,
    })
}

fn enforce_check(
    check: impl Into<String>,
    passed: bool,
    message: impl Into<String>,
) -> EnforceCheck {
    EnforceCheck {
        check: check.into(),
        status: if passed { "pass" } else { "fail" }.to_string(),
        message: message.into(),
    }
}

fn agent_files(client: AgentClient, root: &Path) -> Vec<(PathBuf, String)> {
    let first_required_command = format!("callsieve agent-context {} \"<task>\"", root.display());
    let policy = agent_policy_text(client, &first_required_command);
    let portable_policy = portable_agent_policy_text(client);
    let callsieve_command = callsieve_executable_display();
    match client {
        AgentClient::Codex => vec![
            (
                root.join(".codex/config.toml"),
                format!(
                    "[mcp_servers.callsieve]\ncommand = {}\nargs = [\"mcp\"]\nstartup_timeout_sec = 20\ntool_timeout_sec = 60\n",
                    toml_basic_string(&callsieve_command)
                ),
            ),
            (root.join(".codex/CALLSIEVE.md"), policy),
        ],
        AgentClient::Claude => vec![
            (
                root.join(".mcp.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": {
                        "callsieve": {
                            "type": "stdio",
                            "command": callsieve_command,
                            "args": ["mcp"],
                            "env": {}
                        }
                    }
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            ),
            (root.join("CLAUDE.md"), policy),
        ],
        AgentClient::Copilot => vec![
            (
                root.join(".github/copilot-mcp.json"),
                serde_json::to_string_pretty(&mcp_config_json(&callsieve_command))
                    .unwrap_or_else(|_| "{}".to_string()),
            ),
            (
                root.join(".github/copilot-instructions.md"),
                format!(
                    "{policy}\nLocal Copilot CLI can run the local CallSieve binary. Cloud agent use is template-only unless CallSieve is installed inside that sandbox.\n"
                ),
            ),
            (
                root.join(".github/agents/callsieve-context.agent.md"),
                format!(
                    "---\nname: callsieve-context\ndescription: Use CallSieve context before broad repository search.\n---\n\n{policy}\n"
                ),
            ),
        ],
        AgentClient::OpenCode => vec![
            (
                root.join("opencode.json"),
                opencode_project_json(root, &callsieve_command),
            ),
            (
                root.join(".opencode/CALLSIEVE.md"),
                format!(
                    "{policy}\nThe `.opencode/plugins/callsieve.js` plugin enforces this before broad `grep`, `glob`, `read`, and shell search tools when installed.\n"
                ),
            ),
        ],
        AgentClient::Antigravity => vec![
            (
                root.join(".agents/mcp_config.json"),
                serde_json::to_string_pretty(&mcp_config_json(&callsieve_command))
                    .unwrap_or_else(|_| "{}".to_string()),
            ),
            (
                root.join(".agents/skills/callsieve-context.md"),
                format!(
                    "# CallSieve Context\n\n{policy}\nKeep `GEMINI.md` and `AGENTS.md` compatibility by treating this skill as the local context-first rule.\n"
                ),
            ),
            (root.join(".agents/rules/callsieve.md"), policy),
        ],
        AgentClient::Cursor => vec![
            (
                root.join(".cursor/mcp.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": {
                        "callsieve": {
                            "type": "stdio",
                            "command": callsieve_command,
                            "args": ["mcp"]
                        }
                    }
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            ),
            (root.join(".cursor/rules/callsieve.mdc"), policy),
        ],
        AgentClient::Vscode => vec![
            (
                root.join(".vscode/mcp.json"),
                vscode_mcp_json(root, &callsieve_command),
            ),
            (
                root.join(".github/copilot-instructions.md"),
                format!(
                    "{portable_policy}\nVS Code uses `.vscode/mcp.json` with a workspace `servers.callsieve` stdio MCP server. Keep this project-local; do not mutate user-level VS Code MCP configuration automatically.\n"
                ),
            ),
        ],
        AgentClient::Windsurf => vec![
            (
                root.join(".callsieve/integrations/windsurf-mcp.json"),
                serde_json::to_string_pretty(&mcp_config_json(&callsieve_command))
                    .unwrap_or_else(|_| "{}".to_string()),
            ),
            (
                root.join(".windsurf/rules/callsieve.md"),
                format!(
                    "{portable_policy}\nWindsurf MCP configuration is user-scoped today, so CallSieve writes this repo-local MCP template instead of changing global Windsurf config.\n"
                ),
            ),
        ],
        AgentClient::Continue => vec![
            (
                root.join(".continue/mcpServers/callsieve.yaml"),
                continue_mcp_yaml(&callsieve_command),
            ),
            (root.join(".continue/rules/callsieve.md"), portable_policy),
        ],
        AgentClient::Zed => {
            if zed_project_settings_mergeable(root) {
                vec![(
                    root.join(".zed/settings.json"),
                    zed_settings_json(root, &callsieve_command),
                )]
            } else {
                vec![(
                    root.join(".callsieve/integrations/zed-settings.json"),
                    zed_settings_template_json(&callsieve_command),
                )]
            }
        }
        AgentClient::Junie => vec![
            (
                root.join(".junie/mcp/mcp.json"),
                junie_mcp_json(root, &callsieve_command),
            ),
            (root.join(".junie/guidelines.md"), portable_policy),
        ],
        AgentClient::JetBrains => vec![(
            root.join(".callsieve/integrations/jetbrains-mcp.json"),
            jetbrains_mcp_template_json(&callsieve_command),
        )],
        AgentClient::Amp => vec![
            (
                root.join(".agents/skills/callsieve-context/SKILL.md"),
                amp_skill_markdown(&portable_policy),
            ),
            (
                root.join(".agents/skills/callsieve-context/mcp.json"),
                serde_json::to_string_pretty(&mcp_config_json(&callsieve_command))
                    .unwrap_or_else(|_| "{}".to_string()),
            ),
        ],
        AgentClient::Goose => vec![
            (
                root.join(".callsieve/integrations/goose-config.yaml"),
                goose_config_yaml(&callsieve_command),
            ),
            (
                root.join(".callsieve/integrations/goose-deeplink.txt"),
                goose_deeplink_text(&callsieve_command),
            ),
        ],
        AgentClient::Warp => vec![
            (
                root.join(".callsieve/integrations/warp-mcp.json"),
                serde_json::to_string_pretty(&warp_mcp_json(&callsieve_command))
                    .unwrap_or_else(|_| "{}".to_string()),
            ),
            (
                root.join(".callsieve/integrations/warp-agent.yaml"),
                warp_agent_yaml(&callsieve_command),
            ),
        ],
        AgentClient::Cline => vec![
            (
                root.join(".cline/mcp.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": {
                        "callsieve": {
                            "command": callsieve_command,
                            "args": ["mcp"],
                            "env": {},
                            "disabled": false,
                            "autoApprove": []
                        }
                    }
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            ),
            (root.join(".cline/rules/callsieve.md"), policy.clone()),
            (root.join(".clinerules/callsieve.md"), policy),
        ],
        AgentClient::Zoo | AgentClient::Roo => vec![
            (
                root.join(".roo/mcp.json"),
                serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": {
                        "callsieve": {
                            "command": callsieve_command,
                            "args": ["mcp"],
                            "env": {},
                            "disabled": false
                        }
                    }
                }))
                .unwrap_or_else(|_| "{}".to_string()),
            ),
            (root.join(".roo/rules/callsieve.md"), policy.clone()),
            (root.join(".roo/rules-code/callsieve.md"), policy),
        ],
        AgentClient::Generic => vec![
            (
                root.join(".callsieve/mcp.json"),
                serde_json::to_string_pretty(&mcp_config_json(&callsieve_command))
                    .unwrap_or_else(|_| "{}".to_string()),
            ),
            (
                root.join(".callsieve/mcp.toml"),
                mcp_config_toml(&callsieve_command),
            ),
            (
                root.join(".callsieve/agent-policy.md"),
                format!(
                    "{policy}\nUse `.callsieve/mcp.json` or `.callsieve/mcp.toml` for MCP clients, or the first command above before broad search.\n"
                ),
            ),
        ],
    }
}

fn agent_policy_text(client: AgentClient, first_required_command: &str) -> String {
    let client_name = agent_client_name(client);
    let mcp_hint = match client {
        AgentClient::Codex => {
            "Use the configured Codex MCP server and prefer `callsieve_context` when tools are available."
        }
        AgentClient::Claude => {
            "Use the project MCP server from `.mcp.json` and prefer `callsieve_context` when tools are available."
        }
        AgentClient::Copilot => {
            "Use local Copilot CLI MCP plus Copilot instructions before repository search; cloud agents need CallSieve installed in the sandbox."
        }
        AgentClient::OpenCode => {
            "Use the OpenCode MCP config and CallSieve plugin before `grep`, `glob`, `read`, or shell search tools."
        }
        AgentClient::Antigravity => {
            "Use `.agents/mcp_config.json`, the CallSieve skill, and Antigravity hooks before repository search."
        }
        AgentClient::Cursor => {
            "Use the Cursor MCP config and this rule file before repository search."
        }
        AgentClient::Vscode => {
            "Use VS Code workspace MCP from `.vscode/mcp.json` plus Copilot instructions before repository search."
        }
        AgentClient::Windsurf => {
            "Use the Windsurf rule and repo-local MCP template; copy the template into user-scoped Windsurf MCP settings only after reviewing the local command path."
        }
        AgentClient::Continue => "Use the Continue MCP block and rule before repository search.",
        AgentClient::Zed => {
            "Use Zed `context_servers.callsieve` from project settings or the generated fallback template before repository search."
        }
        AgentClient::Junie => {
            "Use Junie project MCP config and guidelines before repository search."
        }
        AgentClient::JetBrains => {
            "JetBrains AI Assistant setup is documentation-only here; use the generated MCP template manually, or use `--client junie` for Junie."
        }
        AgentClient::Amp => "Use the Amp skill and bundled MCP config before repository search.",
        AgentClient::Goose => {
            "Use the generated Goose extension template before repository search; CallSieve does not mutate user Goose config automatically."
        }
        AgentClient::Warp => {
            "Use the generated Warp local MCP and cloud-agent templates before repository search. Cloud agents need a runtime that can execute the local `callsieve` binary."
        }
        AgentClient::Cline => {
            "Use the Cline MCP server, call `callsieve_context`, and follow this rule before search tools."
        }
        AgentClient::Zoo => {
            "Use the Zoo MCP server, call `callsieve_context`, and follow `.roo` rule files before search tools."
        }
        AgentClient::Roo => {
            "Use the Zoo-compatible `.roo` MCP server, call `callsieve_context`, and follow rules before search tools."
        }
        AgentClient::Generic => "Use `callsieve mcp` for MCP clients when available.",
    };

    format!(
        "CallSieve policy for {client_name}:\n\
1. First command for every coding task: `{first_required_command}`.\n\
2. {mcp_hint}\n\
3. Treat `retrieval_cost.retrieval_model_tokens = 0` as retrieval-only; returned context still counts when read.\n\
4. Read the returned `read_first` files and review `context.sel.next` before broad grep, rg, repository-wide search, or repeated file reads.\n\
5. Use `instruction.x.o/n/next`, MCP `callsieve_focus`, `callsieve_related`, and `callsieve_tests`, or CLI `callsieve focus`, `callsieve related`, and `callsieve tests`, for targeted local detail before broad search.\n\
6. Grep only if the context packet and local expansion are insufficient, and preserve why they were insufficient in the task notes or trace.\n\
7. For audited sessions, run `callsieve enforce <repo> --client {client_name} --trace <trace.json> --strict`.\n"
    )
}

fn portable_agent_policy_text(client: AgentClient) -> String {
    let client_name = agent_client_name(client);
    format!(
        "CallSieve policy for {client_name}:\n\
1. Before broad grep, rg, repository-wide search, or repeated file reads, call the CallSieve MCP `callsieve_context` tool when it is available.\n\
2. If MCP tools are unavailable, run `callsieve agent-context <repo> \"<task>\"` from this workspace.\n\
3. Treat `retrieval_cost.retrieval_model_tokens = 0` as retrieval-only; returned context still counts when read.\n\
4. Read the returned `read_first` files first and review `context.sel.next`.\n\
5. Use `instruction.x.o/n/next`, MCP `callsieve_focus`, `callsieve_related`, and `callsieve_tests`, or CLI `callsieve focus`, `callsieve related`, and `callsieve tests`, before broad search.\n\
6. Grep only if the context packet and local expansion are insufficient, and record why they were insufficient in task notes or trace.\n"
    )
}

fn agent_client_warnings(client: AgentClient) -> Vec<String> {
    let mut warnings = Vec::new();
    if matches!(client, AgentClient::Roo) {
        warnings.push(
            "`roo` is deprecated as a CallSieve client value; generated files are Zoo-compatible `.roo/*` files. Prefer `--client zoo` for new setup."
                .to_string(),
        );
    }
    if matches!(client, AgentClient::JetBrains) {
        warnings.push(
            "JetBrains AI Assistant setup is docs/template only; use `--client junie` for Junie project MCP and guidelines."
                .to_string(),
        );
    }
    if matches!(client, AgentClient::Warp) {
        warnings.push(
            "Warp cloud-agent MCP is template-only unless the Warp/Oz execution environment can run the local `callsieve` binary."
                .to_string(),
        );
    }
    warnings
}

fn agent_client_warnings_for_root(client: AgentClient, root: &Path) -> Vec<String> {
    let mut warnings = agent_client_warnings(client);
    if matches!(client, AgentClient::Zed) && !zed_project_settings_mergeable(root) {
        warnings.push(
            "Existing `.zed/settings.json` is not mergeable JSON; CallSieve writes `.callsieve/integrations/zed-settings.json` as a template instead of overwriting it."
                .to_string(),
        );
    }
    warnings
}

fn read_json_object_or_empty(path: &Path) -> serde_json::Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}))
}

fn nested_object<'a>(
    value: &'a mut serde_json::Value,
    key: &str,
) -> &'a mut serde_json::Map<String, serde_json::Value> {
    let object = value
        .as_object_mut()
        .expect("template root should be a JSON object");
    let entry = object
        .entry(key.to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !entry.is_object() {
        *entry = serde_json::json!({});
    }
    entry
        .as_object_mut()
        .expect("template field should be a JSON object")
}

fn vscode_mcp_json(root: &Path, callsieve_command: &str) -> String {
    let path = root.join(".vscode/mcp.json");
    let mut value = read_json_object_or_empty(&path);
    nested_object(&mut value, "servers").insert(
        "callsieve".to_string(),
        serde_json::json!({
            "type": "stdio",
            "command": callsieve_command,
            "args": ["mcp"],
            "env": {}
        }),
    );
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn zed_project_settings_mergeable(root: &Path) -> bool {
    let path = root.join(".zed/settings.json");
    if !path.exists() {
        return true;
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .is_some_and(|value| value.is_object())
}

fn zed_settings_json(root: &Path, callsieve_command: &str) -> String {
    let path = root.join(".zed/settings.json");
    let mut value = read_json_object_or_empty(&path);
    nested_object(&mut value, "context_servers").insert(
        "callsieve".to_string(),
        serde_json::json!({
            "command": callsieve_command,
            "args": ["mcp"],
            "env": {}
        }),
    );
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn zed_settings_template_json(callsieve_command: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "context_servers": {
            "callsieve": {
                "command": callsieve_command,
                "args": ["mcp"],
                "env": {}
            }
        }
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn junie_mcp_json(root: &Path, callsieve_command: &str) -> String {
    let path = root.join(".junie/mcp/mcp.json");
    let mut value = read_json_object_or_empty(&path);
    nested_object(&mut value, "mcpServers").insert(
        "callsieve".to_string(),
        serde_json::json!({
            "command": callsieve_command,
            "args": ["mcp"],
            "env": {}
        }),
    );
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn jetbrains_mcp_template_json(callsieve_command: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "callsieve": {
                "command": callsieve_command,
                "args": ["mcp"],
                "env": {}
            }
        },
        "notes": [
            "JetBrains AI Assistant MCP setup is manual/template-only here.",
            "Use `callsieve agent-setup <repo> --client junie` for Junie project MCP support.",
            "Before broad search, use callsieve_context, read context.sel.next, then use instruction.x.o/n/next, callsieve_focus, callsieve_related, and callsieve_tests for local detail."
        ]
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn warp_mcp_json(callsieve_command: &str) -> serde_json::Value {
    serde_json::json!({
        "callsieve": {
            "command": callsieve_command,
            "args": ["mcp"],
            "env": {}
        }
    })
}

fn yaml_double_quoted(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }
    format!("\"{escaped}\"")
}

fn continue_mcp_yaml(callsieve_command: &str) -> String {
    format!(
        "name: CallSieve\nversion: {}\nschema: v1\nmcpServers:\n  - name: CallSieve\n    type: stdio\n    command: {}\n    args:\n      - \"mcp\"\n    env: {{}}\n",
        env!("CARGO_PKG_VERSION"),
        yaml_double_quoted(callsieve_command)
    )
}

fn goose_config_yaml(callsieve_command: &str) -> String {
    format!(
        "extensions:\n  callsieve:\n    name: callsieve\n    type: stdio\n    enabled: true\n    command: {}\n    args:\n      - \"mcp\"\n    env: {{}}\n",
        yaml_double_quoted(callsieve_command)
    )
}

fn goose_deeplink_text(callsieve_command: &str) -> String {
    format!(
        "CallSieve Goose extension template\n\nCommand: {callsieve_command}\nArgs: mcp\n\nUse `.callsieve/integrations/goose-config.yaml` as the reviewed local template. Call callsieve_context first, review context.sel.next, then use instruction.x.o/n/next, callsieve_focus, callsieve_related, and callsieve_tests before broad grep. CallSieve does not mutate user Goose config automatically.\n"
    )
}

fn warp_agent_yaml(callsieve_command: &str) -> String {
    format!(
        "name: callsieve-local-agent\nsystem_prompt: \"Use callsieve_context before broad repository search or repeated file reads. Review context.sel.next, then use instruction.x.o/n/next plus callsieve_focus, callsieve_related, and callsieve_tests for local detail before grep.\"\nmcp_servers:\n  callsieve:\n    command: {}\n    args:\n      - \"mcp\"\n    env: {{}}\n# Cloud agents need an environment where this local callsieve binary is installed and runnable.\n",
        yaml_double_quoted(callsieve_command)
    )
}

fn amp_skill_markdown(policy: &str) -> String {
    format!(
        "# CallSieve Context\n\nUse this skill when working in this repository with Amp.\n\n{policy}\nThe bundled `mcp.json` starts `callsieve mcp` as a local stdio MCP server. Do not use broad repository search until the CallSieve context packet and local expansion commands are insufficient.\n"
    )
}

fn opencode_project_json(root: &Path, callsieve_command: &str) -> String {
    let path = root.join("opencode.json");
    let mut value = fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));

    let object = value
        .as_object_mut()
        .expect("opencode config should be a JSON object");
    let mcp = object.entry("mcp").or_insert_with(|| serde_json::json!({}));
    if !mcp.is_object() {
        *mcp = serde_json::json!({});
    }
    if let Some(mcp_object) = mcp.as_object_mut() {
        mcp_object.insert(
            "callsieve".to_string(),
            serde_json::json!({
                "type": "local",
                "command": [callsieve_command, "mcp"],
                "enabled": true
            }),
        );
    }

    let instruction_path = ".opencode/CALLSIEVE.md";
    match object.get_mut("instructions") {
        Some(instructions) if instructions.is_array() => {
            if let Some(array) = instructions.as_array_mut()
                && !array.iter().any(|value| value == instruction_path)
            {
                array.push(serde_json::json!(instruction_path));
            }
        }
        Some(instructions) if instructions.is_string() => {
            let existing = instructions
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .map(|value| serde_json::json!(value));
            let mut array = existing.into_iter().collect::<Vec<_>>();
            if !array.iter().any(|value| value == instruction_path) {
                array.push(serde_json::json!(instruction_path));
            }
            *instructions = serde_json::json!(array);
        }
        _ => {
            object.insert(
                "instructions".to_string(),
                serde_json::json!([instruction_path]),
            );
        }
    }

    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

fn zoo_roomodes_json() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "customModes": [
            {
                "slug": "callsieve-code",
                "name": "CallSieve Code",
                "role": "Use CallSieve context and local expansion before broad repository search.",
                "groups": ["read", "edit", "browser", "command", "mcp"],
                "customInstructions": "Call callsieve_context or run callsieve agent-context before broad grep, repository-wide search, or repeated file reads. Read read_first and context.sel.next, then use instruction.x.o/n/next plus callsieve_focus, callsieve_related, and callsieve_tests, or CLI callsieve focus, callsieve related, and callsieve tests, before grep."
            }
        ]
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn callsieve_executable_display() -> String {
    if let Some(path) = stable_callsieve_executable() {
        return path.display().to_string();
    }

    env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("callsieve"))
        .display()
        .to_string()
}

fn stable_callsieve_executable() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CALLSIEVE_MCP_COMMAND").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }

    let binary_name = if cfg!(windows) {
        "callsieve.exe"
    } else {
        "callsieve"
    };

    if let Some(path) = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("bin").join(binary_name))
        .filter(|path| path.is_file())
    {
        return Some(path);
    }

    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(home_var)
        .map(PathBuf::from)
        .map(|home| home.join(".cargo").join("bin").join(binary_name))
        .filter(|path| path.is_file())
}

fn toml_basic_string(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character => escaped.push(character),
        }
    }
    format!("\"{escaped}\"")
}

fn agent_client_name(client: AgentClient) -> &'static str {
    match client {
        AgentClient::Codex => "codex",
        AgentClient::Claude => "claude",
        AgentClient::Copilot => "copilot",
        AgentClient::OpenCode => "opencode",
        AgentClient::Antigravity => "antigravity",
        AgentClient::Cursor => "cursor",
        AgentClient::Vscode => "vscode",
        AgentClient::Windsurf => "windsurf",
        AgentClient::Continue => "continue",
        AgentClient::Zed => "zed",
        AgentClient::Junie => "junie",
        AgentClient::JetBrains => "jetbrains",
        AgentClient::Amp => "amp",
        AgentClient::Goose => "goose",
        AgentClient::Warp => "warp",
        AgentClient::Cline => "cline",
        AgentClient::Zoo => "zoo",
        AgentClient::Roo => "roo",
        AgentClient::Generic => "generic",
    }
}

fn callsieve_dir(root: &Path) -> PathBuf {
    root.join(store::json_store::INDEX_DIR)
}

fn missing_daemon_state(root: &Path) -> DaemonState {
    DaemonState {
        status: "missing".to_string(),
        root: root_label(root),
        mode: "none".to_string(),
        pid: 0,
        lsp: false,
        interval_ms: 0,
        started_at: 0,
        last_indexed_at: 0,
        last_change_at: 0,
        index_generation: 0,
        stale_files: 0,
        last_error: Some("daemon state file is missing".to_string()),
    }
}

fn write_guard_trace(path: &Path, task: &str, context: &query::ContextOutput) -> Result<String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let context_value = serde_json::to_value(context)?;
    let files_read: Vec<String> = context_value
        .get("read_first")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            file.get("f")
                .or_else(|| file.get("file"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_string)
        .collect();
    let context_tokens = serde_json::to_string(&context_value)
        .map(|json| json.len().div_ceil(4))
        .unwrap_or_default();
    let trace = serde_json::json!({
        "tasks": [
            {
                "id": "guarded-task",
                "task": task,
                "expected_files": files_read,
                "session": {
                    "baseline": {
                        "grep_commands": 0,
                        "file_reads": 0,
                        "tokens": 0,
                        "commands": [],
                        "files_read": []
                    },
                    "callsieve": {
                        "grep_commands": 0,
                        "file_reads": context_value
                            .get("read_first")
                            .and_then(serde_json::Value::as_array)
                            .map(Vec::len)
                            .unwrap_or_default(),
                        "tokens": context_tokens,
                        "commands": [
                            "callsieve guard",
                            "callsieve agent-context"
                        ],
                        "files_read": files_read
                    }
                }
            }
        ]
    });

    fs::write(path, serde_json::to_vec_pretty(&trace)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path.display().to_string())
}

fn run_shim_command(
    root: &Path,
    tool: ShimTool,
    strict: bool,
    args: &[String],
) -> Result<ShimRunOutput> {
    let pattern = extract_search_pattern(tool, args);
    let context = if let Some(pattern) = pattern.as_deref() {
        let (index, index_load_ms, refreshed) = load_fresh_index_timed(root)?;
        let mut context = query::build_context_with_options(
            root,
            &index,
            pattern,
            query::DEFAULT_AGENT_CONTEXT_LIMIT,
            2,
            true,
            false,
        )?;
        context.add_index_load_time(index_load_ms);
        if refreshed {
            context.add_warning("rebuilt missing or stale CallSieve index before context");
        }
        Some(compact_context_value_for_prompt(&context)?)
    } else {
        None
    };
    let shim_command = format!("{} {}", shim_tool_name(tool), args.join(" "));
    let shim_event = if strict {
        Some(record_shim_grep_event(
            root,
            Some(&shim_command),
            pattern.as_deref().unwrap_or("<unparsed>"),
        )?)
    } else {
        None
    };
    let passthrough = run_passthrough_command(root, tool, args)?;

    Ok(ShimRunOutput {
        command: "shim-run",
        root: root_label(root),
        tool: shim_tool_name(tool),
        args: args.to_vec(),
        pattern,
        context,
        shim_event,
        passthrough,
    })
}

fn extract_search_pattern(tool: ShimTool, args: &[String]) -> Option<String> {
    match tool {
        ShimTool::Rg => extract_rg_pattern(args),
        ShimTool::Grep => extract_grep_pattern(args),
    }
}

fn extract_rg_pattern(args: &[String]) -> Option<String> {
    let mut index = 0;
    let mut after_double_dash = false;
    while index < args.len() {
        let arg = &args[index];
        if after_double_dash {
            return Some(arg.clone());
        }
        if arg == "--" {
            after_double_dash = true;
            index += 1;
            continue;
        }
        if arg == "-e" || arg == "--regexp" {
            return args.get(index + 1).cloned();
        }
        if let Some(pattern) = arg.strip_prefix("-e")
            && !pattern.is_empty()
        {
            return Some(pattern.to_string());
        }
        if rg_option_consumes_next(arg) {
            index += 2;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(arg.clone());
    }
    None
}

fn extract_grep_pattern(args: &[String]) -> Option<String> {
    let mut index = 0;
    let mut after_double_dash = false;
    while index < args.len() {
        let arg = &args[index];
        if after_double_dash {
            return Some(arg.clone());
        }
        if arg == "--" {
            after_double_dash = true;
            index += 1;
            continue;
        }
        if arg == "-e" || arg == "--regexp" {
            return args.get(index + 1).cloned();
        }
        if let Some(pattern) = arg.strip_prefix("-e")
            && !pattern.is_empty()
        {
            return Some(pattern.to_string());
        }
        if grep_option_consumes_next(arg) {
            index += 2;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        return Some(arg.clone());
    }
    None
}

fn rg_option_consumes_next(arg: &str) -> bool {
    matches!(
        arg,
        "-g" | "--glob"
            | "-t"
            | "--type"
            | "-T"
            | "--type-not"
            | "--type-add"
            | "--type-clear"
            | "-f"
            | "--file"
            | "--ignore-file"
            | "--path-separator"
            | "--colors"
            | "--sort"
            | "--sortr"
            | "--engine"
    )
}

fn grep_option_consumes_next(arg: &str) -> bool {
    matches!(
        arg,
        "-f" | "--file" | "-m" | "--max-count" | "-A" | "-B" | "-C"
    )
}

fn run_passthrough_command(root: &Path, tool: ShimTool, args: &[String]) -> Result<RgOutput> {
    let bin_dir = shim_bin_dir(root);
    let tool_name = shim_tool_name(tool);
    let Some(real_command) = resolve_command_excluding_shim(tool_name, &bin_dir) else {
        return Ok(RgOutput {
            status_code: None,
            stdout: String::new(),
            stderr: format!(
                "real {tool_name} was not found outside {}",
                bin_dir.display()
            ),
        });
    };
    let output = ProcessCommand::new(&real_command)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to run real {tool_name}: {real_command}"))?;
    Ok(RgOutput {
        status_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn shim_tool_name(tool: ShimTool) -> &'static str {
    match tool {
        ShimTool::Rg => "rg",
        ShimTool::Grep => "grep",
    }
}

fn record_shim_grep_event(
    root: &Path,
    shim_command: Option<&str>,
    pattern: &str,
) -> Result<serde_json::Value> {
    let path = shim_trace_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut value = if path.is_file() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_else(|| shim_trace_template(root))
    } else {
        shim_trace_template(root)
    };
    let existing_commands = value
        .get("events")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|event| event.get("command").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let context_seen = existing_commands
        .iter()
        .any(|command| is_callsieve_context_command_local(command));
    let policy_violation = !context_seen;
    let grep_command = shim_command
        .filter(|command| !command.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("rg {pattern}"));
    let context_command = format!("callsieve grep {} {:?}", root.display(), pattern);
    let events = value
        .as_object_mut()
        .context("shim trace root must be a JSON object")?
        .entry("events")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .context("shim trace events must be an array")?;
    events.push(serde_json::json!({
        "timestamp": now_unix_seconds(),
        "command": grep_command,
        "files_read": [],
        "tokens": 0,
        "classification": "grep",
        "phase": "callsieve",
        "policy_violation": policy_violation,
        "event_kind": if policy_violation { "grep_before_context" } else { "grep_after_context" }
    }));
    events.push(serde_json::json!({
        "timestamp": now_unix_seconds(),
        "command": context_command,
        "files_read": [],
        "tokens": 0,
        "classification": "callsieve_context",
        "phase": "callsieve"
    }));
    normalize_session_trace(&mut value)?;
    fs::write(&path, serde_json::to_vec_pretty(&value)?)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(serde_json::json!({
        "trace": path.display().to_string(),
        "policy_violation": policy_violation,
        "event_kind": if policy_violation { "grep_before_context" } else { "grep_after_context" },
        "message": if policy_violation {
            "grep shim ran before any context event in this shim trace; context was printed before running real grep"
        } else {
            "grep shim ran after a context event in this shim trace"
        }
    }))
}

fn shim_trace_template(root: &Path) -> serde_json::Value {
    serde_json::json!({
        "metadata": {
            "collection": "shim_trace",
            "repo": root_label(root),
            "strict": true,
            "started_at": now_unix_seconds(),
            "updated_at": now_unix_seconds()
        },
        "task": "project-local grep shim usage",
        "expected_files": [],
        "critical_files": [],
        "baseline": empty_session_metrics(),
        "callsieve": empty_session_metrics(),
        "session": {
            "baseline": empty_session_metrics(),
            "callsieve": empty_session_metrics()
        },
        "events": [],
        "misses": [],
        "policy": {
            "context_first": true,
            "strict_trace_check": true
        }
    })
}

fn install_shim(root: &Path, force: bool, strict: bool) -> Result<ShimOutput> {
    let bin_dir = shim_bin_dir(root);
    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;
    let real_rg = resolve_command_excluding_shim("rg", &bin_dir);
    let real_grep = resolve_command_excluding_shim("grep", &bin_dir);
    let mut files = Vec::new();

    for path in shim_script_paths(&bin_dir, "callsieve") {
        if path.exists() && !force {
            anyhow::bail!(
                "refusing to overwrite {}; pass --force to replace it",
                path.display()
            );
        }
        write_executable_file(&path, &callsieve_launcher_script(&path))
            .with_context(|| format!("failed to write {}", path.display()))?;
        files.push(path.display().to_string());
    }

    for (name, real_command) in [("rg", real_rg), ("grep", real_grep)] {
        let script_files = shim_script_paths(&bin_dir, name);
        for path in script_files {
            if path.exists() && !force {
                anyhow::bail!(
                    "refusing to overwrite {}; pass --force to replace it",
                    path.display()
                );
            }
            write_executable_file(
                &path,
                &shim_script(root, name, real_command.as_deref(), &path, strict),
            )
            .with_context(|| format!("failed to write {}", path.display()))?;
            files.push(path.display().to_string());
        }
    }

    Ok(ShimOutput {
        command: "shim install",
        status: "pass".to_string(),
        root: root_label(root),
        bin_dir: bin_dir.display().to_string(),
        strict,
        trace: strict.then(|| shim_trace_path(root).display().to_string()),
        files,
        path_instruction: shim_path_instruction(&bin_dir),
    })
}

fn shim_doctor(root: &Path) -> ShimDoctorOutput {
    let bin_dir = shim_bin_dir(root);
    let mut checks = Vec::new();
    checks.push(enforce_check(
        "shim_bin_dir",
        bin_dir.is_dir(),
        if bin_dir.is_dir() {
            "shim bin directory exists"
        } else {
            "shim bin directory is missing"
        },
    ));

    for file in ["callsieve", "rg", "grep"] {
        let paths = shim_script_paths(&bin_dir, file);
        let installed = paths.iter().any(|path| path.is_file());
        checks.push(enforce_check(
            format!("shim_file:{file}"),
            installed,
            if installed {
                "shim wrapper exists"
            } else {
                "shim wrapper is missing"
            },
        ));
    }

    let on_path = shim_dir_on_path(&bin_dir);
    checks.push(enforce_check(
        "path_contains_shim_dir",
        on_path,
        if on_path {
            "shim bin directory is on PATH"
        } else {
            "prepend shim bin directory to PATH before running agents"
        },
    ));

    let status = if checks.iter().all(|check| check.status == "pass") {
        "pass"
    } else {
        "fail"
    }
    .to_string();

    ShimDoctorOutput {
        command: "shim doctor",
        status,
        root: root_label(root),
        checks,
        path_instruction: shim_path_instruction(&bin_dir),
    }
}

fn uninstall_shim(root: &Path) -> Result<ShimOutput> {
    let bin_dir = shim_bin_dir(root);
    let mut removed = Vec::new();
    for name in ["callsieve", "rg", "grep"] {
        for path in shim_script_paths(&bin_dir, name) {
            if path.is_file() {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to remove {}", path.display()))?;
                removed.push(path.display().to_string());
            }
        }
    }

    Ok(ShimOutput {
        command: "shim uninstall",
        status: "pass".to_string(),
        root: root_label(root),
        bin_dir: bin_dir.display().to_string(),
        strict: false,
        trace: None,
        files: removed,
        path_instruction: shim_path_instruction(&bin_dir),
    })
}

fn shim_bin_dir(root: &Path) -> PathBuf {
    callsieve_dir(root).join("bin")
}

fn shim_trace_path(root: &Path) -> PathBuf {
    callsieve_dir(root).join("shim-trace.json")
}

fn shim_files_installed(root: &Path) -> bool {
    let bin_dir = shim_bin_dir(root);
    ["callsieve", "rg", "grep"].iter().all(|file| {
        shim_script_paths(&bin_dir, file)
            .iter()
            .any(|path| path.is_file())
    })
}

fn shim_dir_on_path(bin_dir: &Path) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|path| same_path(&path, bin_dir)))
        .unwrap_or(false)
}

fn shim_script_paths(bin_dir: &Path, name: &str) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![bin_dir.join(format!("{name}.cmd"))]
    } else {
        vec![bin_dir.join(name)]
    }
}

fn shim_script(
    root: &Path,
    name: &str,
    _real_command: Option<&str>,
    path: &Path,
    strict: bool,
) -> String {
    let root = root.display().to_string();
    if path.extension().and_then(|extension| extension.to_str()) == Some("cmd") {
        let strict_args = if strict { " --strict" } else { "" };
        return format!(
            "@echo off\r\nsetlocal\r\nset CALLSIEVE_SHIM_ACTIVE=1\r\ncallsieve shim-run \"{root}\" --tool {name}{strict_args} -- %*\r\n"
        );
    }

    let strict_args = if strict { " --strict" } else { "" };
    format!(
        "#!/usr/bin/env sh\nexport CALLSIEVE_SHIM_ACTIVE=1\nexec callsieve shim-run '{}' --tool {}{} -- \"$@\"\n",
        sh_single_quote(&root),
        name,
        strict_args
    )
}

fn callsieve_launcher_script(path: &Path) -> String {
    let exe = callsieve_executable_display();
    if path.extension().and_then(|extension| extension.to_str()) == Some("cmd") {
        let exe = exe.replace('%', "%%");
        return format!("@echo off\r\nsetlocal\r\n\"{exe}\" %*\r\n");
    }

    format!(
        "#!/usr/bin/env sh\nexec '{}' \"$@\"\n",
        sh_single_quote(&exe)
    )
}

fn sh_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn write_executable_file(path: &Path, content: &str) -> Result<()> {
    fs::write(path, content).with_context(|| format!("failed to write {}", path.display()))?;
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to stat {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to chmod {}", path.display()))?;
    }
    Ok(())
}

fn set_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to stat {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to chmod {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn resolve_command_excluding_shim(command: &str, shim_bin: &Path) -> Option<String> {
    let paths = env::var_os("PATH")?;
    let names = command_candidate_names(command);
    for dir in env::split_paths(&paths) {
        if same_path(&dir, shim_bin) {
            continue;
        }
        for name in &names {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path.display().to_string());
            }
        }
    }
    None
}

fn command_candidate_names(command: &str) -> Vec<String> {
    if cfg!(windows) {
        let extensions = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut names = vec![command.to_string()];
        names.extend(
            extensions
                .split(';')
                .map(str::trim)
                .filter(|extension| !extension.is_empty())
                .map(|extension| format!("{command}{}", extension.to_ascii_lowercase())),
        );
        names
    } else {
        vec![command.to_string()]
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left == right
}

fn shim_path_instruction(bin_dir: &Path) -> String {
    if cfg!(windows) {
        format!("prepend {} to PATH for agent shells", bin_dir.display())
    } else {
        format!(
            "export PATH=\"{}:$PATH\" before running agents",
            bin_dir.display()
        )
    }
}

fn anonymize_evidence(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                if matches!(
                    key.as_str(),
                    "path"
                        | "root"
                        | "suite_path"
                        | "trace_path"
                        | "policy_trace_path"
                        | "label"
                        | "team"
                        | "team_id"
                        | "team_name"
                        | "case_study"
                        | "case_study_id"
                ) {
                    *value = serde_json::Value::String("<redacted>".to_string());
                } else if matches!(
                    key.as_str(),
                    "suite_paths"
                        | "trace_paths"
                        | "policy_trace_paths"
                        | "teams"
                        | "team_ids"
                        | "case_studies"
                ) {
                    if let Some(values) = value.as_array_mut() {
                        for item in values {
                            *item = serde_json::Value::String("<redacted>".to_string());
                        }
                    } else {
                        *value = serde_json::Value::String("<redacted>".to_string());
                    }
                } else {
                    anonymize_evidence(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                anonymize_evidence(value);
            }
        }
        _ => {}
    }
}

fn evidence_pack_protocol(manifest_json: &str) -> String {
    let protocol = serde_json::from_str::<serde_json::Value>(manifest_json)
        .ok()
        .and_then(|value| {
            value
                .get("protocol")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    value
                        .get("audit")
                        .and_then(|audit| audit.get("protocol"))
                        .and_then(|protocol| protocol.get("evidence_standard"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
        })
        .unwrap_or_else(|| "pilot-proof".to_string());

    format!(
        "{protocol}; local repos only; collect observed traces; run strict policy checks; publish aggregate JSON"
    )
}

fn run_rg(root: &Path, pattern: &str) -> Result<RgOutput> {
    let output = ProcessCommand::new("rg")
        .arg(pattern)
        .arg(root)
        .output()
        .context("failed to run rg")?;

    Ok(RgOutput {
        status_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_all_commands() {
        std::thread::Builder::new()
            .name("parse-all-commands".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(parses_all_commands_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn competitive_report_evidence_requires_savings_and_strict_trace_policy() {
        let mut value = serde_json::json!({
            "summary": {
                "tasks": 2,
                "first_correct_file_rate_at_k": 0.5,
                "expected_file_recall": 0.25,
                "baseline_context_payload_tokens_estimate": 100,
                "callsieve_context_payload_tokens_estimate": 21,
                "context_payload_reduction": {
                    "context_payload_reduction_percent": 79.0
                },
                "total_avoided_grep_commands": 3,
                "total_avoided_file_reads": 4,
                "session_trace": null
            }
        });
        let passing_policy = serde_json::json!({
            "sessions": 1,
            "violations": 0,
            "context_first_compliant": true
        });

        let evidence = competitive_callsieve_evidence(&value, Some(&passing_policy));
        assert_eq!(evidence.default_packet_tokens, 11);
        assert!(!evidence.trace_or_receipt_proof_available);

        value["summary"]["observed_session"] = serde_json::json!({
            "sessions": 1,
            "token_savings": 1000,
            "avoided_grep_commands": 3,
            "avoided_file_reads": 4,
            "files_still_missed": 0,
            "critical_files_still_missed": 0
        });

        let failing_policy = serde_json::json!({
            "sessions": 1,
            "violations": 1,
            "context_first_compliant": false
        });
        let evidence = competitive_callsieve_evidence(&value, Some(&failing_policy));
        assert!(!evidence.trace_or_receipt_proof_available);
        assert_eq!(evidence.strict_trace_policy_violations, 1);
        assert!(!evidence.context_first_compliant);

        let evidence = competitive_callsieve_evidence(&value, Some(&passing_policy));
        assert!(evidence.trace_or_receipt_proof_available);
        assert_eq!(evidence.strict_trace_policy_sessions, 1);
        assert_eq!(evidence.strict_trace_policy_violations, 0);
        assert!(evidence.context_first_compliant);
    }

    #[test]
    fn public_broad_read_guardrail_requires_smaller_context_packet() {
        let targets = PublicProofTargets {
            minimum_broad_read_context_payload_reduction_percent: 10.0,
            ..PublicProofTargets::default()
        };
        let passing_value = serde_json::json!({
            "summary": {
                "tasks": 1,
                "first_correct_file_rate_at_k": 1.0,
                "expected_file_recall": 1.0,
                "baseline_context_payload_tokens_estimate": 1000,
                "callsieve_context_payload_tokens_estimate": 250,
                "context_payload_reduction": {
                    "context_payload_reduction_percent": 75.0
                },
                "total_avoided_grep_commands": 0,
                "total_avoided_file_reads": 4
            }
        });
        let passing_evidence = competitive_callsieve_evidence(&passing_value, None);
        let guardrail = public_broad_read_guardrail(&passing_evidence, &targets);
        assert_eq!(guardrail.status, "pass");
        assert_eq!(guardrail.context_payload_tokens_saved, 750);

        let failing_value = serde_json::json!({
            "summary": {
                "tasks": 1,
                "first_correct_file_rate_at_k": 1.0,
                "expected_file_recall": 1.0,
                "baseline_context_payload_tokens_estimate": 1000,
                "callsieve_context_payload_tokens_estimate": 950,
                "context_payload_reduction": {
                    "context_payload_reduction_percent": 5.0
                },
                "total_avoided_grep_commands": 0,
                "total_avoided_file_reads": 0
            }
        });
        let failing_evidence = competitive_callsieve_evidence(&failing_value, None);
        let guardrail = public_broad_read_guardrail(&failing_evidence, &targets);
        assert_eq!(guardrail.status, "needs_work");
    }

    #[test]
    fn public_context_packet_guardrail_requires_agent_useful_packet_fields() {
        let passing_value = serde_json::json!({
            "summary": {
                "tasks": 2,
                "first_correct_file_rate_at_k": 1.0,
                "expected_file_recall": 1.0,
                "baseline_context_payload_tokens_estimate": 1000,
                "callsieve_context_payload_tokens_estimate": 250,
                "context_payload_reduction": {
                    "context_payload_reduction_percent": 75.0
                },
                "packet_quality": {
                    "tasks": 2,
                    "selected_files": 5,
                    "files_with_symbols": 4,
                    "selected_symbols": 7,
                    "files_with_snippets": 2,
                    "snippets": 2,
                    "files_with_related_tests": 2,
                    "related_tests": 3,
                    "files_with_blast_radius": 3,
                    "blast_radius_hints": 6,
                    "files_with_call_graph_hints": 2,
                    "call_graph_hints": 4,
                    "files_with_non_unknown_risk": 5,
                    "files_with_selection_reasons": 5,
                    "selection_reasons": 8,
                    "files_with_selection_confidence": 5,
                    "selection_signals": 4,
                    "next_file_hints": 2,
                    "focus_targets": 5,
                    "relationship_followup_targets": 1,
                    "test_followup_targets": 1
                }
            }
        });
        let passing_evidence = competitive_callsieve_evidence(&passing_value, None);
        let guardrail = public_context_packet_guardrail(&passing_evidence);
        assert_eq!(guardrail.status, "pass");
        assert_eq!(guardrail.selected_symbols, 7);
        assert_eq!(guardrail.call_graph_hints, 4);
        assert_eq!(guardrail.relationship_followup_targets, 1);

        let missing_call_graph_value = serde_json::json!({
            "summary": {
                "tasks": 1,
                "first_correct_file_rate_at_k": 1.0,
                "expected_file_recall": 1.0,
                "baseline_context_payload_tokens_estimate": 1000,
                "callsieve_context_payload_tokens_estimate": 250,
                "context_payload_reduction": {
                    "context_payload_reduction_percent": 75.0
                },
                "packet_quality": {
                    "tasks": 1,
                    "selected_files": 2,
                    "files_with_symbols": 2,
                    "selected_symbols": 2,
                    "files_with_snippets": 0,
                    "snippets": 0,
                    "files_with_related_tests": 1,
                    "related_tests": 1,
                    "files_with_blast_radius": 2,
                    "blast_radius_hints": 2,
                    "files_with_call_graph_hints": 0,
                    "call_graph_hints": 0,
                    "files_with_non_unknown_risk": 2,
                    "files_with_selection_reasons": 2,
                    "selection_reasons": 2,
                    "files_with_selection_confidence": 2,
                    "selection_signals": 2,
                    "next_file_hints": 1,
                    "focus_targets": 2,
                    "relationship_followup_targets": 1,
                    "test_followup_targets": 1
                }
            }
        });
        let failing_evidence = competitive_callsieve_evidence(&missing_call_graph_value, None);
        let guardrail = public_context_packet_guardrail(&failing_evidence);
        assert_eq!(guardrail.status, "needs_work");
    }

    #[test]
    fn public_agent_native_check_artifact_recomputes_measured_preflight() {
        let temp = tempfile::tempdir().unwrap();
        let task_log = temp.path().join("agent-native-task-log.json");
        let transcript = temp.path().join("agent-native-transcript.json");
        let check_artifact = temp.path().join("agent-native-check.json");
        fs::write(
            &task_log,
            r#"{
  "tasks": [
    {
      "id": "native-hit",
      "task": "find createSession token behavior",
      "expected_files": ["src/auth/session.ts"],
      "agent_native_files": ["src/auth/session.ts"],
      "callsieve_files": ["src/auth/session.ts"],
      "agent_native_context_tokens": 20000,
      "callsieve_packet_tokens": 1000
    }
  ]
}"#,
        )
        .unwrap();
        fs::write(
            &transcript,
            r#"{"tool":"fixture-native-search","task_ids":["native-hit"]}"#,
        )
        .unwrap();

        let task_log_bytes = fs::read(&task_log).unwrap();
        let transcript_bytes = fs::read(&transcript).unwrap();
        let forged_check = serde_json::json!({
            "command": "agent-native-check",
            "schema_version": 1,
            "status": "pass",
            "mode": "measured",
            "tasks_path": "agent-native-task-log.json",
            "task_count": 1,
            "tasks_with_expected_files": 1,
            "tasks_with_callsieve_files": 1,
            "tasks_with_callsieve_packet_tokens": 1,
            "tasks_with_agent_native_files": 1,
            "tasks_with_agent_native_context_tokens": 1,
            "tasks_ready_for_mode": 1,
            "source_artifact_evidence": [
                {
                    "path": "agent-native-task-log.json",
                    "role": "task_log",
                    "bytes": task_log_bytes.len(),
                    "hash": indexer::stable_content_hash(&task_log_bytes)
                },
                {
                    "path": "agent-native-transcript.json",
                    "role": "agent_native_transcript_or_export",
                    "bytes": transcript_bytes.len(),
                    "hash": indexer::stable_content_hash(&transcript_bytes)
                }
            ],
            "issues": [],
            "next_step": "Run agent-native-baseline with the same task log and source artifacts."
        });
        fs::write(
            &check_artifact,
            serde_json::to_vec_pretty(&forged_check).unwrap(),
        )
        .unwrap();
        let input = PublicProofArtifactInput {
            id: "agent-native-check".to_string(),
            path: check_artifact,
            kind: "json_artifact".to_string(),
            description: "forged measured preflight".to_string(),
            required: true,
        };

        let (status, validation, source_hashes) =
            public_agent_native_check_artifact_status(&input, true);

        assert_eq!(status, "needs_work");
        assert!(source_hashes.is_empty());
        assert!(validation.contains("do not recompute"));
        assert!(validation.contains("recording_status"));
    }

    #[test]
    fn claude_collector_command_summary_uses_stdin_prompt() {
        let command = claude_observed_command_summary(
            Path::new("benchmarks/github-axum"),
            "change Router route method handling and path routing",
            PilotSessionMode::Callsieve,
            "sonnet",
            "0.50",
            &["Glob".to_string(), "Grep".to_string(), "Read".to_string()],
        );

        assert!(command.contains("claude -p --input-format text"));
        assert!(command.contains("<callsieve agent-context benchmarks/github-axum"));
        assert!(!command.contains("claude -p \""));
    }

    #[test]
    fn codex_hook_smoke_validation_accepts_codex_permission_schema() {
        validate_hook_smoke_output(
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"CallSieve needs context before broad repo search."}}"#,
            HookSmokeExpectation::CodexPermissionDecision {
                event: "PreToolUse",
                expected: "deny",
            },
        )
        .unwrap();
    }

    #[test]
    fn codex_hook_smoke_validation_rejects_allow_decision_for_noop() {
        validate_hook_smoke_output(
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse"}}"#,
            HookSmokeExpectation::CodexNoop {
                event: "PreToolUse",
            },
        )
        .unwrap();

        let error = validate_hook_smoke_output(
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","permissionDecisionReason":"allowed"}}"#,
            HookSmokeExpectation::CodexNoop {
                event: "PreToolUse",
            },
        )
        .unwrap_err();
        assert_eq!(
            error,
            "unsupported hookSpecificOutput.permissionDecision in Codex PreToolUse no-op response"
        );
    }

    #[test]
    fn codex_hook_smoke_validation_rejects_unsupported_top_level_fields() {
        let expectation = HookSmokeExpectation::CodexPermissionDecision {
            event: "PreToolUse",
            expected: "deny",
        };

        let suppress = validate_hook_smoke_output(
            r#"{"suppressOutput":true,"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"blocked"}}"#,
            expectation,
        )
        .unwrap_err();
        assert_eq!(
            suppress,
            "unsupported top-level suppressOutput in Codex PreToolUse response"
        );

        let decision = validate_hook_smoke_output(
            r#"{"decision":"deny","hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"blocked"}}"#,
            expectation,
        )
        .unwrap_err();
        assert_eq!(
            decision,
            "unsupported top-level decision in Codex PreToolUse response"
        );

        let reason = validate_hook_smoke_output(
            r#"{"reason":"blocked","hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"blocked"}}"#,
            expectation,
        )
        .unwrap_err();
        assert_eq!(
            reason,
            "unsupported top-level reason in Codex PreToolUse response"
        );
    }

    fn parses_all_commands_inner() {
        Cli::try_parse_from(["callsieve", "index", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "index", ".", "--lsp"]).unwrap();
        Cli::try_parse_from(["callsieve", "symbols", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "symbol", ".", "UserService"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "focus",
            ".",
            "--file",
            "src/query/mod.rs",
            "--symbol",
            "build_context",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "related", ".", "--file", "src/query/mod.rs"]).unwrap();
        Cli::try_parse_from(["callsieve", "tests", ".", "--file", "src/query/mod.rs"]).unwrap();
        Cli::try_parse_from(["callsieve", "query", ".", "where is auth handled?"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "query",
            ".",
            "where is auth handled?",
            "--why-debug",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "context", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "context",
            ".",
            "change token expiry",
            "--why-debug",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "context",
            ".",
            "change token expiry",
            "--profile",
            "skim",
            "--token-budget",
            "1200",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "context",
            ".",
            "change token expiry",
            "--format",
            "markdown",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "agent-context", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "agent-context",
            ".",
            "change token expiry",
            "--profile",
            "full",
            "--snippets-per-file",
            "1",
            "--token-budget",
            "4000",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "agent-context",
            ".",
            "change token expiry",
            "--why-debug",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "agent-context",
            ".",
            "change token expiry",
            "--format",
            "markdown",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "demo", ".", "--task", "change token expiry"]).unwrap();
        Cli::try_parse_from(["callsieve", "demo", ".", "--lsp"]).unwrap();
        Cli::try_parse_from(["callsieve", "memory-clear", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "benchmark", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from(["callsieve", "benchmark-suite", ".", "benchmarks/tasks.json"])
            .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "bench-public",
            "benchmarks/public/manifest.json",
            "benchmarks/public/repos",
            "--compare",
            "--k",
            "5",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "bench-run",
            "benchmarks/public/manifest-50.json",
            "--workdir",
            "/tmp/csbench",
            "--compare",
            "--limit",
            "8",
            "--out",
            "benchmarks/public/results/compare-50.json",
            "--resume",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "eval-retrieval",
            "benchmarks/retrieval-fixtures.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "perf-report",
            ".",
            "--tasks",
            "benchmarks/retrieval-fixtures.json",
            "--iterations",
            "3",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "trace-summary",
            "benchmarks/session-trace.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "session-start",
            ".",
            "change token expiry",
            "--client",
            "codex",
            "--model",
            "gpt-5-codex",
            "--trace",
            "benchmarks/observed-session.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "session-event",
            "benchmarks/observed-session.json",
            "--command",
            "callsieve agent-context . \"change token expiry\"",
            "--files-read",
            "src/main.rs",
            "--tokens",
            "200",
            "--phase",
            "callsieve",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "session-finish",
            "benchmarks/observed-session.json",
            "--out",
            "benchmarks/observed-summary.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "trace-replay",
            ".",
            "benchmarks/tasks.json",
            "benchmarks/session-trace.json",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "trace-check", "benchmarks/session-trace.json"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "trace-check",
            "benchmarks/session-trace.json",
            "--strict",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "benchmark-report", "benchmarks/manifest.json"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "competitive-report",
            "benchmarks/competitive.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "repo-pack-baseline",
            ".",
            "--id",
            "full-repo-pack-proxy",
            "--out",
            "benchmarks/public/results/full-repo-pack-proxy.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "agent-native-baseline",
            "benchmarks/public/results/cursor-public-requests-tasks.json",
            "--id",
            "cursor-public-requests",
            "--tool",
            "Cursor native codebase search",
            "--k",
            "5",
            "--measurement-command",
            "manual approved Cursor run",
            "--source-artifact",
            "benchmarks/public/results/cursor-public-requests-transcript.json",
            "--measurement-note",
            "pinned public Requests tasks",
            "--out",
            "benchmarks/public/results/cursor-public-requests.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "agent-native-check",
            "benchmarks/public/results/cursor-public-requests-tasks.json",
            "--mode",
            "measured",
            "--source-artifact",
            "benchmarks/public/results/cursor-public-requests-transcript.json",
            "--out",
            "benchmarks/public/results/cursor-public-requests-check.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "agent-native-protocol",
            "--out",
            "benchmarks/public/results/agent-native-protocol.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "agent-native-template",
            ".",
            "benchmarks/public/results/cursor-public-requests-tasks.json",
            "--k",
            "5",
            "--out",
            "benchmarks/public/results/cursor-public-requests-tasks.template.json",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "benchmark-doctor", "benchmarks/manifest.json"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "proof-rehearsal",
            "--fix",
            "--resume",
            "--retry-count",
            "2",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "proof-rehearsal", "--preflight"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "proof-rehearsal",
            "--collect-ollama",
            "--ollama-limit",
            "2",
            "--ollama-context-limit",
            "24",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "setup-observed-codex-oss-50", "--force"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "setup-observed-codex-oss-50",
            "--manifest",
            "benchmarks/evidence/observed-codex-oss-50.local.json",
            "--bootstrap-repos",
            "--skip-repo-check",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "setup-observed-claude-oss-50",
            "--manifest",
            "benchmarks/evidence/observed-claude-oss-50.local.json",
            "--model",
            "claude-opus-4-8",
            "--skip-repo-check",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "record-observed-session",
            "--manifest",
            "benchmarks/evidence/observed-claude-oss-50.local.json",
            "--client",
            "claude",
            "--model",
            "claude-opus-4-8",
            "--task-id",
            "auth",
            "--mode",
            "baseline",
            "--command",
            "claude -p \"fix auth\" --output-format json",
            "--usage-json",
            "benchmarks/evidence/claude-auth-baseline.json",
            "--files-read",
            "src/main.rs",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "collect-claude-observed-session",
            "--manifest",
            "benchmarks/evidence/observed-claude-oss-50.local.json",
            "--task-id",
            "auth",
            "--mode",
            "callsieve",
            "--model",
            "claude-opus-4-8",
            "--max-budget-usd",
            "0.50",
            "--context-limit",
            "4",
            "--snippets-per-file",
            "0",
            "--allowed-tool",
            "Glob",
            "--allowed-tool",
            "Grep",
            "--allowed-tool",
            "Read",
            "--dry-run",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "proof-sprint",
            "init",
            "benchmarks/evidence/proof-sprint.local.json",
            "--client",
            "claude",
            "--sessions",
            "10",
            "--model",
            "claude-opus-4-8",
            "--force",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "proof-sprint",
            "status",
            "benchmarks/evidence/proof-sprint.local.json",
            "--json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "proof-sprint",
            "collect",
            "benchmarks/evidence/proof-sprint.local.json",
            "--task-id",
            "auth",
            "--mode",
            "baseline",
            "--max-budget-usd",
            "0.50",
            "--dry-run",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "proof-sprint",
            "run",
            "benchmarks/evidence/proof-sprint.local.json",
            "--resume",
            "--limit",
            "2",
            "--max-budget-usd",
            "0.50",
            "--dry-run",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "proof-sprint",
            "finalize",
            "benchmarks/evidence/proof-sprint.local.json",
            "--out",
            "benchmarks/evidence/proof.local.json",
            "--limit",
            "24",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "record-codex-observed-session",
            "--task-id",
            "auth",
            "--mode",
            "callsieve",
            "--command",
            "callsieve agent-context . \"change token expiry\"",
            "--tokens",
            "200",
            "--files-read",
            "src/main.rs",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "pilot-init",
            "benchmarks/evidence/pilot.json",
            "--sessions",
            "50",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "pilot-task",
            "add",
            "benchmarks/evidence/pilot.json",
            ".",
            "change token expiry",
            "--id",
            "auth",
            "--expected-file",
            "src/main.rs",
            "--critical-file",
            "src/main.rs",
            "--external",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "pilot-task",
            "reject",
            "benchmarks/evidence/pilot.json",
            "--task-id",
            "auth",
            "--reason",
            "operator learned answer during paired run",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "pilot-run",
            "benchmarks/evidence/pilot.json",
            "--task-id",
            "auth",
            "--mode",
            "callsieve",
            "--command",
            "callsieve agent-context . \"change token expiry\"",
            "--files-read",
            "src/main.rs",
            "--tokens",
            "200",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "pilot-collect-ollama",
            "benchmarks/evidence/pilot.json",
            "--model",
            "qwen2.5-coder:7b",
            "--limit",
            "10",
            "--context-limit",
            "24",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "pilot-collect-lm-studio",
            "benchmarks/evidence/pilot.json",
            "--model",
            "qwen3-coder-next",
            "--base-url",
            "http://127.0.0.1:1234/v1",
            "--limit",
            "10",
            "--context-limit",
            "24",
            "--max-tokens",
            "256",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "pilot-qa", "benchmarks/evidence/pilot.json"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "pilot-finalize",
            "benchmarks/evidence/pilot.json",
            "--out",
            "benchmarks/evidence/proof.json",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "pilot-report", "benchmarks/manifest.json"]).unwrap();
        Cli::try_parse_from(["callsieve", "proof-report", "benchmarks/manifest.json"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "enterprise-proof-report",
            "benchmarks/manifest.json",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "pilot-doctor", "benchmarks/manifest.json"]).unwrap();
        Cli::try_parse_from(["callsieve", "evidence-pack", "benchmarks/manifest.json"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "policy-check",
            "benchmarks/session-trace.json",
            "--strict",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "mcp"]).unwrap();
        Cli::try_parse_from(["callsieve", "mcp-config", ".", "--format", "json"]).unwrap();
        Cli::try_parse_from(["callsieve", "mcp-config", ".", "--format", "toml"]).unwrap();
        Cli::try_parse_from(["callsieve", "mcp-registry-manifest"]).unwrap();
        Cli::try_parse_from(["callsieve", "mcp-registry-manifest", "--out", "server.json"])
            .unwrap();
        Cli::try_parse_from(["callsieve", "mcp-contract"]).unwrap();
        Cli::try_parse_from(["callsieve", "mcp-contract", "--out", "mcp-contract.json"]).unwrap();
        Cli::try_parse_from(["callsieve", "status", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon", ".", "--once"]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon", ".", "--background", "--lsp"]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon-status", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon-stop", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "watch", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "watch", ".", "--lsp"]).unwrap();
        Cli::try_parse_from(["callsieve", "agent-setup", ".", "--client", "codex"]).unwrap();
        for client in [
            "copilot",
            "opencode",
            "antigravity",
            "cursor",
            "vscode",
            "windsurf",
            "continue",
            "zed",
            "junie",
            "jetbrains",
            "amp",
            "goose",
            "warp",
            "cline",
            "zoo",
            "roo",
        ] {
            Cli::try_parse_from(["callsieve", "agent-setup", ".", "--client", client]).unwrap();
            Cli::try_parse_from(["callsieve", "doctor", ".", "--client", client, "--strict"])
                .unwrap();
            Cli::try_parse_from(["callsieve", "enforce", ".", "--client", client, "--strict"])
                .unwrap();
            Cli::try_parse_from([
                "callsieve",
                "hook",
                "install",
                ".",
                "--client",
                client,
                "--strict",
                "--force",
            ])
            .unwrap();
        }
        Cli::try_parse_from([
            "callsieve",
            "bootstrap",
            ".",
            "--client",
            "generic",
            "--strict",
            "--force",
            "--lsp",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "doctor", ".", "--client", "generic"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "doctor",
            ".",
            "--client",
            "generic",
            "--fix",
            "--strict",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "setup-agent", "codex", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "setup-agent", "roo", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "setup-agent", "vscode", "."]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "codex-bootstrap",
            ".",
            "--model",
            "gpt-5-codex",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "codex-hooks", "install", ".", "--strict"]).unwrap();
        Cli::try_parse_from(["callsieve", "codex-hooks", "doctor", ".", "--strict"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "codex-hooks",
            "doctor",
            ".",
            "--strict",
            "--smoke",
            "--fix",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "codex-hooks", "trust-ack", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "codex-hooks", "uninstall", "."]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "codex-hook",
            "user-prompt-submit",
            ".",
            "--strict",
            "--limit",
            "6",
            "--snippets-per-file",
            "1",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "codex-hook", "pre-tool-use", ".", "--strict"]).unwrap();
        Cli::try_parse_from(["callsieve", "codex-hook", "post-tool-use", ".", "--strict"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "codex-hook",
            "permission-request",
            ".",
            "--strict",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "codex-hook", "stop", ".", "--strict"]).unwrap();
        Cli::try_parse_from(["callsieve", "claude-hooks", "install", ".", "--strict"]).unwrap();
        Cli::try_parse_from(["callsieve", "claude-hooks", "doctor", ".", "--strict"]).unwrap();
        Cli::try_parse_from(["callsieve", "claude-hooks", "uninstall", "."]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "claude-hook",
            "user-prompt-submit",
            ".",
            "--strict",
            "--limit",
            "6",
            "--snippets-per-file",
            "1",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "claude-hook", "pre-tool-use", ".", "--strict"]).unwrap();
        Cli::try_parse_from(["callsieve", "claude-hook", "post-tool-use", ".", "--strict"])
            .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "claude-hook",
            "permission-request",
            ".",
            "--strict",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "claude-hook", "stop", ".", "--strict"]).unwrap();
        for hooks_command in [
            "copilot-hooks",
            "opencode-hooks",
            "antigravity-hooks",
            "cline-hooks",
        ] {
            Cli::try_parse_from(["callsieve", hooks_command, "install", ".", "--strict"]).unwrap();
            Cli::try_parse_from(["callsieve", hooks_command, "doctor", ".", "--strict"]).unwrap();
            Cli::try_parse_from(["callsieve", hooks_command, "uninstall", "."]).unwrap();
        }
        for hook_command in [
            "copilot-hook",
            "opencode-hook",
            "antigravity-hook",
            "cline-hook",
        ] {
            Cli::try_parse_from([
                "callsieve",
                hook_command,
                "user-prompt-submit",
                ".",
                "--strict",
                "--limit",
                "6",
                "--snippets-per-file",
                "1",
            ])
            .unwrap();
            Cli::try_parse_from(["callsieve", hook_command, "pre-tool-use", ".", "--strict"])
                .unwrap();
            Cli::try_parse_from(["callsieve", hook_command, "post-tool-use", ".", "--strict"])
                .unwrap();
            Cli::try_parse_from([
                "callsieve",
                hook_command,
                "permission-request",
                ".",
                "--strict",
            ])
            .unwrap();
            Cli::try_parse_from(["callsieve", hook_command, "stop", ".", "--strict"]).unwrap();
        }
        Cli::try_parse_from([
            "callsieve",
            "editor-hook",
            ".",
            "--editor",
            "cursor",
            "--force",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "guard", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "begin",
            ".",
            "change token expiry",
            "--client",
            "codex",
            "--trace-out",
            "benchmarks/begin-trace.json",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "begin",
            ".",
            "change token expiry",
            "--client",
            "codex",
            "--trace-out",
            "benchmarks/proof-begin-trace.json",
            "--proof-trace",
        ])
        .unwrap();
        Cli::try_parse_from([
            "callsieve",
            "codex-session",
            ".",
            "change token expiry",
            "--trace-out",
            "benchmarks/codex-session.json",
            "--model",
            "gpt-5-codex",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "enforce", ".", "--client", "generic"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "enforce",
            ".",
            "--client",
            "generic",
            "--require-shim",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "shim", "install", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "shim", "install", ".", "--strict"]).unwrap();
        Cli::try_parse_from(["callsieve", "shim", "doctor", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "shim", "uninstall", "."]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "hook",
            "install",
            ".",
            "--client",
            "generic",
            "--strict",
            "--force",
            "--lsp",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "hook", "doctor", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "hook", "uninstall", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "grep", ".", "createSession"]).unwrap();
        Cli::try_parse_from([
            "callsieve",
            "shim-run",
            ".",
            "--tool",
            "rg",
            "--strict",
            "--",
            "-n",
            "createSession",
            "src",
        ])
        .unwrap();
        Cli::try_parse_from(["callsieve", "stats", "."]).unwrap();
    }

    #[test]
    fn parses_ollama_verbose_token_counts_after_stripping_ansi() {
        let raw = "\u{1b}[?25l{\"ok\":true}\u{1b}[?25h\r\n\
total duration:       3.36s\n\
prompt eval count:    44 token(s)\n\
prompt eval duration: 87ms\n\
eval count:           21 token(s)\n\
eval duration:        235ms\n";
        let clean = strip_ansi(raw);
        assert!(!clean.contains('\u{1b}'));
        let (prompt, eval) = parse_ollama_verbose_counts(&clean).unwrap();
        assert_eq!(prompt, 44);
        assert_eq!(eval, 21);
    }

    #[test]
    fn parses_lm_studio_chat_endpoint() {
        let endpoint = openai_chat_endpoint("http://127.0.0.1:1234/v1").unwrap();
        assert_eq!(endpoint.host, "127.0.0.1");
        assert_eq!(endpoint.port, 1234);
        assert_eq!(endpoint.path, "/v1/chat/completions");

        let endpoint = openai_chat_endpoint("http://localhost:1234/v1/chat/completions").unwrap();
        assert_eq!(endpoint.host, "localhost");
        assert_eq!(endpoint.port, 1234);
        assert_eq!(endpoint.path, "/v1/chat/completions");
    }

    #[test]
    fn parses_http_response_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 12\r\n\r\n{\"ok\":true}";
        let (status, body) = parse_http_response(response).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "{\"ok\":true}");
    }

    #[test]
    fn decodes_chunked_http_response_body() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"ok\":t\r\n4\r\nrue}\r\n0\r\n\r\n";
        let (status, body) = parse_http_response(response).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, "{\"ok\":true}");
    }
}

#[cfg(test)]
mod setup_auto_tests {
    use super::*;

    #[test]
    fn detects_agents_from_home_dirs_binaries_and_extensions() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let bin = temp.path().join("bin");
        let apps = temp.path().join("Applications");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(home.join(".vscode/extensions/saoudrizwan.claude-dev-3.0.0")).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("codex"), "#!/bin/sh\n").unwrap();
        fs::create_dir_all(apps.join("Zed.app")).unwrap();

        let detected = detect_agent_clients(&home, &[bin], &apps);
        let names: Vec<&str> = detected
            .iter()
            .map(|(client, _)| agent_client_name(*client))
            .collect();

        assert!(names.contains(&"claude"), "{names:?}");
        assert!(names.contains(&"codex"), "{names:?}");
        assert!(names.contains(&"zed"), "{names:?}");
        assert!(names.contains(&"cline"), "{names:?}");
        assert!(!names.contains(&"cursor"), "{names:?}");
        assert!(!names.contains(&"goose"), "{names:?}");
    }

    #[test]
    fn detection_reports_evidence_kind() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(home.join(".claude")).unwrap();

        let detected = detect_agent_clients(&home, &[], Path::new("/nonexistent"));

        assert_eq!(detected.len(), 1);
        assert!(detected[0].1.starts_with("directory:"), "{}", detected[0].1);
    }

    #[test]
    fn setup_auto_installs_hooks_for_hook_capable_clients() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path();
        fs::write(repo.join("main.rs"), "fn main() {}\n").unwrap();

        let output = setup_auto_with_detected(
            repo,
            true,
            false,
            vec![(AgentClient::Claude, "test".to_string())],
        )
        .unwrap();

        assert_eq!(output.configured.len(), 1, "{:?}", output.warnings);
        let files = &output.configured[0].files;
        assert!(
            files.iter().any(|file| file.contains("hook")
                || file.contains("settings")
                || file.contains(".claude/")),
            "expected hook files alongside setup files: {files:?}"
        );
    }

    #[test]
    fn setup_auto_dry_run_configures_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let output = setup_auto_with_detected(
            temp.path(),
            false,
            true,
            vec![(AgentClient::Claude, "test".to_string())],
        )
        .unwrap();

        assert!(output.configured.is_empty());
        assert_eq!(output.detected.len(), 1);
        assert!(fs::read_dir(temp.path()).unwrap().next().is_none());
    }
}

#[cfg(test)]
mod index_transfer_tests {
    use super::*;

    fn build_and_save(root: &Path) {
        let index = indexer::build_index(root).unwrap();
        store::json_store::save_index(root, &index).unwrap();
    }

    #[test]
    fn export_import_round_trip_is_fresh_on_a_second_checkout() {
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        fs::write(source.path().join("b.rs"), "pub fn beta() { alpha(); }\n").unwrap();
        build_and_save(source.path());

        let export = source.path().join("team-index.json");
        let exported = index_export(source.path(), &export).unwrap();
        assert!(exported.files >= 2);

        // Second checkout: same contents, different mtimes.
        let clone = tempfile::tempdir().unwrap();
        fs::write(clone.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        fs::write(clone.path().join("b.rs"), "pub fn beta() { alpha(); }\n").unwrap();

        let imported = index_import(clone.path(), &export, false).unwrap();
        assert_eq!(imported.files_mismatched, 0);
        assert_eq!(imported.files_missing, 0);
        assert_eq!(imported.fingerprint, exported.fingerprint);

        let index = store::json_store::load_index(clone.path()).unwrap();
        let status = query::index_status(clone.path(), Some(&index));
        assert!(
            status.is_fresh(),
            "imported index must pass the freshness check"
        );
    }

    #[test]
    fn import_rejects_a_drifted_checkout_without_allow_partial() {
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("a.rs"), "pub fn alpha() {}\n").unwrap();
        build_and_save(source.path());
        let export = source.path().join("team-index.json");
        index_export(source.path(), &export).unwrap();

        let clone = tempfile::tempdir().unwrap();
        fs::write(clone.path().join("a.rs"), "pub fn alpha() { changed(); }\n").unwrap();

        let error = index_import(clone.path(), &export, false).unwrap_err();
        assert!(error.to_string().contains("differ"), "{error}");
    }
}

#[cfg(test)]
mod hook_path_tests {
    use super::*;

    #[test]
    fn repo_relative_handles_symlinked_and_relative_roots() {
        let temp = tempfile::tempdir().unwrap();
        // On macOS the tempdir itself is behind the /tmp -> /private/tmp
        // symlink, so the UNcanonicalized handle is exactly the failure case.
        let root = temp.path().to_path_buf();
        fs::write(root.join("session.ts"), "x").unwrap();
        let absolute = root.canonicalize().unwrap().join("session.ts");

        assert_eq!(
            hook_repo_relative(absolute.to_str().unwrap(), &root),
            "session.ts"
        );
        assert_eq!(hook_repo_relative("session.ts", &root), "session.ts");
    }

    #[test]
    fn confirmed_reads_keep_only_real_repo_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        fs::write(root.join("real.ts"), "x").unwrap();
        let mut state = CodexHookState {
            context_seen: true,
            ..CodexHookState::default()
        };

        record_confirmed_reads(
            &mut state,
            &root,
            &format!(
                "cat -n {} nonexistent.ts | grep pattern",
                root.canonicalize().unwrap().join("real.ts").display()
            ),
        );

        assert_eq!(state.confirmed_reads, vec!["real.ts"]);
    }

    #[test]
    fn claude_hooks_config_matches_edit_tools_for_post_tool_use() {
        let temp = tempfile::tempdir().unwrap();
        let (_, _, _files) = write_claude_hooks_files(temp.path(), false, true, 6, 1).unwrap();
        let settings = fs::read_to_string(temp.path().join(".claude/settings.local.json"))
            .or_else(|_| fs::read_to_string(temp.path().join(".claude/settings.json")))
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&settings).unwrap();
        let post = value["hooks"]["PostToolUse"]
            .as_array()
            .expect("PostToolUse hooks registered");
        let matchers: Vec<&str> = post
            .iter()
            .filter_map(|entry| entry["matcher"].as_str())
            .collect();
        assert!(
            matchers
                .iter()
                .any(|matcher| matcher.contains("Edit") && matcher.contains("Write")),
            "PostToolUse matcher must include edit tools, got {matchers:?}"
        );
    }

    #[test]
    fn public_proof_reads_single_arm_mode_a_reports_as_lexical() {
        let value = serde_json::json!({
            "manifest": "benchmarks/public/manifest-50.json",
            "aggregate": {
                "evaluated": 50,
                "first_correct_file_rate_at_k": 0.64
            },
            "issues": [
                {
                    "id": "hit",
                    "selected_files_count": 8,
                    "first_correct_file_rate_at_k": 1.0
                },
                {
                    "id": "miss",
                    "selected_files_count": 6,
                    "first_correct_file_rate_at_k": 0.0
                }
            ]
        });

        assert_eq!(
            public_report_manifest_path(&value).as_deref(),
            Some("benchmarks/public/manifest-50.json")
        );
        assert_eq!(public_report_arm_rate(&value, "lexical"), 0.64);
        assert_eq!(public_report_arm_rate(&value, "hybrid"), 0.0);
        assert_eq!(average_selected_files_for_arm(&value, "lexical"), 7.0);
        assert_eq!(
            public_report_misses(&value, "lexical", 8),
            vec!["miss".to_string()]
        );
    }
}

#[cfg(test)]
mod hook_summary_tests {
    use super::*;

    fn state_with(packets: usize, tokens: usize, files: usize) -> CodexHookState {
        CodexHookState {
            context_packets: packets,
            context_tokens: tokens,
            context_files: files,
            ..CodexHookState::default()
        }
    }

    #[test]
    fn no_summary_when_no_context_served() {
        assert_eq!(hook_session_summary_message(&state_with(0, 0, 0), 0), None);
    }

    #[test]
    fn summary_reports_observed_counts_only() {
        let message = hook_session_summary_message(&state_with(3, 540, 14), 0).unwrap();
        assert_eq!(
            message,
            "CallSieve: 3 context packets (~540 tokens, 14 read-first files) served this session; local retrieval spent 0 AI model tokens. Receipt: `callsieve receipt <repo> --latest`."
        );
        assert!(
            !message.contains("saved"),
            "summary must not make savings claims: {message}"
        );
    }

    #[test]
    fn summary_reports_learned_associations() {
        let message = hook_session_summary_message(&state_with(1, 194, 6), 3).unwrap();
        assert!(message.contains("Learned 3 read associations"), "{message}");
        assert!(!message.contains("saved"), "{message}");
    }

    #[test]
    fn summary_uses_singular_forms() {
        let message = hook_session_summary_message(&state_with(1, 194, 1), 0).unwrap();
        assert!(message.contains("1 context packet ("), "{message}");
        assert!(message.contains("1 read-first file)"), "{message}");
    }
}
