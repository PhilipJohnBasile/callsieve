use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::{indexer, output, query, store};

const REHEARSAL_RETRIEVAL_FIXTURES: &str = "benchmarks/retrieval-fixtures.json";
const REHEARSAL_EXTERNAL_MANIFEST: &str = "benchmarks/external-github-manifest.example.json";
const OBSERVED_CODEX_OSS_50_MANIFEST: &str = "benchmarks/evidence/observed-codex-oss-50.local.json";
const REHEARSAL_REPORT_LIMIT: usize = 24;
const REHEARSAL_SNIPPETS_PER_FILE: usize = 2;

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

#[derive(Debug, Parser)]
#[command(
    name = "callsieve",
    version,
    about = "Local-first codebase retrieval for AI coding agents"
)]
pub struct Cli {
    #[arg(long, global = true)]
    verbose: bool,

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

        #[arg(long, value_enum, default_value_t = AgentOutputFormat::Json)]
        format: AgentOutputFormat,
    },

    /// Build an agent-ready context packet agents should request before grep.
    AgentContext {
        path: PathBuf,
        task: String,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        /// Include structured scoring components for ranking diagnostics.
        #[arg(long)]
        why_debug: bool,

        #[arg(long, value_enum, default_value_t = AgentOutputFormat::Json)]
        format: AgentOutputFormat,
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

        #[arg(long = "dry-run")]
        dry_run: bool,
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

    /// Generate Codex-first local bootstrap files without global PATH/profile mutation.
    CodexBootstrap {
        path: PathBuf,

        #[arg(long)]
        model: String,

        #[arg(long)]
        force: bool,
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

        #[arg(long, default_value_t = 8)]
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

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum AgentClient {
    Codex,
    Claude,
    Cursor,
    Cline,
    Roo,
    Generic,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum McpConfigFormat {
    Json,
    Toml,
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

#[derive(Debug, Serialize)]
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
    action: &'static str,
    guidance: &'static str,
    grep_policy: &'static str,
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
    next_step: String,
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
    context: query::ContextOutput,
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
struct PilotCollectOllamaOutput {
    command: &'static str,
    manifest: String,
    model: String,
    requested_sessions: usize,
    collected_sessions: usize,
    skipped_sessions: usize,
    observed_sessions: usize,
    qa_status: String,
    sessions: Vec<PilotCollectOllamaSessionOutput>,
}

#[derive(Debug, Serialize)]
struct PilotCollectOllamaSessionOutput {
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

struct PilotPromptPlan {
    command: String,
    files_read: Vec<String>,
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

#[derive(Debug, Serialize, serde::Deserialize)]
struct DaemonState {
    status: String,
    root: String,
    mode: String,
    pid: u32,
    lsp: bool,
    interval_ms: u64,
    started_at: u64,
    last_indexed_at: u64,
    last_change_at: u64,
    index_generation: u64,
    stale_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
}

#[derive(Debug)]
struct DaemonIndexSnapshot {
    last_indexed_at: u64,
    index_generation: u64,
    stale_files: usize,
    last_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct DaemonOutput {
    command: &'static str,
    state: DaemonState,
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
    launchers: Vec<String>,
    first_required_command: String,
    path_instruction: String,
    policy: &'static str,
}

#[derive(Debug, Serialize)]
struct HookDoctorOutput {
    command: &'static str,
    status: String,
    root: String,
    checks: Vec<EnforceCheck>,
    shim: ShimDoctorOutput,
    path_instruction: String,
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
    context: Option<query::ContextOutput>,
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

    match cli.command {
        Command::Index { path, lsp, .. } => {
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
            format,
        } => {
            let (index, index_load_ms) = load_index_timed(&path)?;
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
            print_context_output(&output, format)?;
        }
        Command::AgentContext {
            path,
            task,
            limit,
            snippets_per_file,
            why_debug,
            format,
        } => {
            let (index, index_load_ms) = load_index_timed(&path)?;
            let mut context = query::build_context_with_options(
                &path,
                &index,
                &task,
                limit,
                snippets_per_file,
                true,
                why_debug,
            )?;
            context.add_index_load_time(index_load_ms);
            let memory = query::task_memory_for_context(&path, &context, now_unix_seconds())?;
            let output = AgentContextOutput {
                instruction: AgentContextInstruction {
                    action: "read_first_before_grep",
                    guidance: "Read these files first; grep only if insufficient.",
                    grep_policy: "grep_only_if_context_is_insufficient",
                },
                memory,
                context,
            };
            print_agent_context_output(&output, format)?;
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
        } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::benchmark_context(
                &path,
                &index,
                &task,
                limit,
                snippets_per_file,
                !no_snippets,
            )?;
            output::json::print(&output)?;
        }
        Command::BenchmarkSuite {
            path,
            tasks,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let index = store::json_store::load_index(&path)?;
            let tasks_json = fs::read_to_string(&tasks)
                .with_context(|| format!("failed to read benchmark suite: {}", tasks.display()))?;
            let suite: query::BenchmarkSuiteInput = serde_json::from_str(&tasks_json)
                .with_context(|| format!("failed to parse benchmark suite: {}", tasks.display()))?;
            let output = query::benchmark_suite(
                &path,
                &index,
                suite,
                limit,
                snippets_per_file,
                !no_snippets,
            )?;
            output::json::print(&output)?;
        }
        Command::EvalRetrieval {
            manifest,
            limit,
            snippets_per_file,
            no_snippets,
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
            let output = query::eval_retrieval(
                &path,
                &index,
                suite,
                limit,
                snippets_per_file,
                !no_snippets,
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
            tokens,
            phase,
        } => {
            let output = session_event(&trace, &event_command, files_read, tokens, phase)?;
            output::json::print(&output)?;
        }
        Command::SessionFinish { trace, out } => {
            let output = session_finish(&trace, &out)?;
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
                dry_run,
            )?;
            output::json::print(&output)?;
        }
        Command::RecordCodexObservedSession {
            manifest,
            task_id,
            mode,
            event_command,
            tokens,
            files_read,
            dry_run,
        } => {
            let output = record_codex_observed_session(
                &manifest,
                &task_id,
                mode,
                &event_command,
                tokens,
                files_read,
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
            tokens,
        } => {
            let output = pilot_run(
                &manifest,
                &task_id,
                mode,
                &event_command,
                files_read,
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
            let index = store::json_store::load_index(&path)?;
            let context =
                query::build_context(&path, &index, &task, limit, snippets_per_file, true)?;
            let trace_path = trace_out
                .as_ref()
                .map(|path| write_guard_trace(path, &task, &context))
                .transpose()?;
            let output = GuardOutput {
                command: "guard",
                root: root_label(&path),
                task,
                policy: "read_first_before_grep; audit broad grep/read with trace-check --strict",
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
            limit,
            snippets_per_file,
        } => {
            let output = begin_task(
                &path,
                &task,
                client,
                trace_out.as_deref(),
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
            let index = store::json_store::load_index(&path)?;
            let include_snippets = !no_snippets;
            let context = query::build_context(
                &path,
                &index,
                &task,
                limit,
                snippets_per_file,
                include_snippets,
            )?;
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
            let index = store::json_store::load_index(&path)?;
            let context =
                query::build_context(&path, &index, &pattern, limit, snippets_per_file, true)?;
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
    }

    Ok(())
}

fn load_index_timed(path: &Path) -> Result<(store::CodeIndex, u64)> {
    let started = Instant::now();
    let index = store::json_store::load_index(path)?;
    Ok((index, duration_ms(started.elapsed())))
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
            format!("callsieve mcp-config {} --format json", path.display()),
            "callsieve proof-rehearsal --preflight".to_string(),
        ],
        warnings: index.warnings.clone(),
    })
}

fn print_context_output(output: &query::ContextOutput, format: AgentOutputFormat) -> Result<()> {
    match format {
        AgentOutputFormat::Json => output::json::print(output),
        AgentOutputFormat::Markdown => {
            let value = serde_json::to_value(output)?;
            println!(
                "{}",
                context_markdown(&value, "grep_only_if_context_is_insufficient")
            );
            Ok(())
        }
    }
}

fn print_agent_context_output(
    output: &AgentContextOutput,
    format: AgentOutputFormat,
) -> Result<()> {
    match format {
        AgentOutputFormat::Json => output::json::print(output),
        AgentOutputFormat::Markdown => {
            let value = serde_json::to_value(output)?;
            let context = value.get("context").unwrap_or(&value);
            let grep_policy = value
                .get("instruction")
                .and_then(|instruction| instruction.get("grep_policy"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("grep_only_if_context_is_insufficient");
            println!("{}", context_markdown(context, grep_policy));
            Ok(())
        }
    }
}

fn context_markdown(context: &serde_json::Value, grep_policy: &str) -> String {
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

    for file in files {
        let rank = json_usize(&file, &["rank"]).unwrap_or_default();
        let path = json_string(&file, &["file"]).unwrap_or_else(|| "<unknown>".to_string());
        let language = json_string(&file, &["language"]).unwrap_or_default();
        let score = json_usize(&file, &["score"]).unwrap_or_default();
        output.push_str(&format!("\n{rank}. `{path}`"));
        if !language.is_empty() {
            output.push_str(&format!(" ({language}, score {score})"));
        }
        output.push('\n');

        if let Some(why) = file.get("why").and_then(serde_json::Value::as_array)
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

        if let Some(symbols) = file.get("symbols").and_then(serde_json::Value::as_array)
            && !symbols.is_empty()
        {
            let names = symbols
                .iter()
                .filter_map(|symbol| {
                    let name = symbol.get("name")?.as_str()?;
                    let kind = symbol.get("kind").and_then(serde_json::Value::as_str)?;
                    Some(format!("{kind} `{name}`"))
                })
                .collect::<Vec<_>>();
            if !names.is_empty() {
                output.push_str(&format!("   Symbols: {}\n", names.join(", ")));
            }
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

fn json_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(str::to_string)
}

fn json_usize(value: &serde_json::Value, path: &[&str]) -> Option<usize> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_u64().map(|number| number as usize)
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

fn record_codex_observed_session(
    manifest: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    event_command: &str,
    tokens: usize,
    files_read: Vec<String>,
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
        dry_run,
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
    if files_read.is_empty() {
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
    );
    let pilot_run = if dry_run {
        None
    } else {
        Some(serde_json::to_value(pilot_run_with_token_evidence(
            manifest,
            task_id,
            mode,
            event_command,
            files_read.clone(),
            token_input.tokens,
            Some(&token_evidence),
        )?)?)
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
    limit: usize,
    snippets_per_file: usize,
) -> Result<BeginOutput> {
    let (index, index_load_ms) = load_index_timed(root)?;
    let mut context = query::build_context(root, &index, task, limit, snippets_per_file, true)?;
    context.add_index_load_time(index_load_ms);
    let context_value = serde_json::to_value(&context)?;
    let files_read = context_read_first_files(&context_value);
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
            files_read.clone(),
            files_read.clone(),
        )?;
        let event = session_event(
            trace,
            &command,
            files_read,
            Some(tokens),
            Some(SessionPhase::Callsieve),
        )?
        .event;
        (Some(trace.display().to_string()), event)
    } else {
        (
            None,
            serde_json::json!({
                "timestamp": now_unix_seconds(),
                "command": command,
                "files_read": files_read,
                "tokens": tokens,
                "classification": "callsieve_context",
                "phase": "callsieve"
            }),
        )
    };

    let next_step = if let Some(trace_path) = trace_path.as_deref() {
        format!(
            "Read read_first files before broad grep; audit with `callsieve trace-check {trace_path} --strict`."
        )
    } else {
        "Read read_first files before broad grep; pass --trace-out to record an audited trace."
            .to_string()
    };

    Ok(BeginOutput {
        command: "begin",
        root: root_label(root),
        client: agent_client_name(client).to_string(),
        task: task.to_string(),
        policy: "context_first; read returned files before broad grep or repeated file reads",
        next_step,
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
        policy: "Call callsieve_context before broad grep, rg, repository search, or repeated file reads.",
    })
}

fn default_agent_model(client: AgentClient) -> &'static str {
    match client {
        AgentClient::Codex => "gpt-5-codex",
        AgentClient::Claude => "claude",
        AgentClient::Cursor => "cursor",
        AgentClient::Cline => "cline",
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

fn daemon_state_is_usable(state: &DaemonState) -> bool {
    !matches!(
        state.status.as_str(),
        "missing" | "stopped" | "stop_requested" | "error"
    ) && state.last_error.is_none()
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

fn session_event(
    trace: &Path,
    command: &str,
    files_read: Vec<String>,
    tokens: Option<usize>,
    phase: Option<SessionPhase>,
) -> Result<SessionEventOutput> {
    session_event_with_token_evidence(trace, command, files_read, tokens, phase, None)
}

fn session_event_with_token_evidence(
    trace: &Path,
    command: &str,
    files_read: Vec<String>,
    tokens: Option<usize>,
    phase: Option<SessionPhase>,
    token_evidence: Option<&serde_json::Value>,
) -> Result<SessionEventOutput> {
    let mut value = read_trace_value(trace)?;
    let phase_name = phase
        .map(session_phase_name)
        .map(str::to_string)
        .unwrap_or_else(|| infer_session_phase(&value, command).to_string());
    let mut event = serde_json::json!({
        "timestamp": now_unix_seconds(),
        "command": command,
        "files_read": files_read,
        "tokens": tokens,
        "classification": classify_session_command(command),
        "phase": phase_name
    });
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

fn session_finish(trace: &Path, out: &Path) -> Result<SessionFinishOutput> {
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
    let summary_value = serde_json::json!({
        "command": "session-finish",
        "trace": trace.display().to_string(),
        "summary": summary,
        "misses": value.get("misses").cloned().unwrap_or_else(|| serde_json::json!([])),
        "token_accounting": value
            .get("token_accounting")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}))
    });

    if let Some(parent) = out.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(out, serde_json::to_vec_pretty(&summary_value)?)
        .with_context(|| format!("failed to write {}", out.display()))?;

    Ok(SessionFinishOutput {
        command: "session-finish",
        trace: trace.display().to_string(),
        out: out.display().to_string(),
        summary,
    })
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
    tokens: usize,
) -> Result<PilotRunOutput> {
    pilot_run_with_token_evidence(
        manifest_path,
        task_id,
        mode,
        command,
        files_read,
        tokens,
        None,
    )
}

fn pilot_run_with_token_evidence(
    manifest_path: &Path,
    task_id: &str,
    mode: PilotSessionMode,
    command: &str,
    files_read: Vec<String>,
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
        Some(tokens),
        Some(phase),
        token_evidence,
    )?;
    session_event_with_token_evidence(
        mode_trace,
        command,
        files_read,
        Some(tokens),
        Some(phase),
        token_evidence,
    )?;
    let finish = session_finish(Path::new(&task.trace_path), Path::new(&task.summary_path))?;
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
) -> Result<PilotCollectOllamaOutput> {
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
    Ok(PilotCollectOllamaOutput {
        command: "pilot-collect-ollama",
        manifest: manifest_path.display().to_string(),
        model: model.to_string(),
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
) -> Result<PilotCollectOllamaSessionOutput> {
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
    let callsieve_files = callsieve_plan.files_read.len();
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

    Ok(PilotCollectOllamaSessionOutput {
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
    let files_read = context_read_first_files(&context_value);
    let compact_context = compact_agent_context_value(&context_value);
    let command = format!(
        "callsieve agent-context {} {:?} --limit {context_limit} --snippets-per-file {snippets_per_file}",
        root.display(),
        task.task
    );
    let mut prompt = String::new();
    prompt.push_str("You are an audited local coding agent CallSieve phase.\n");
    prompt.push_str("Use the CallSieve read-first context below before any broad search.\n");
    prompt.push_str("Return compact JSON with files_read copied from the read_first packet and a one-sentence rationale.\n\n");
    prompt.push_str(&format!("TASK: {}\n", task.task));
    prompt.push_str(&format!("REPO: {}\n", root.display()));
    prompt.push_str(&format!("COMMAND: {command}\n"));
    prompt.push_str("AUDITED_FILES:\n");
    for file in &files_read {
        prompt.push_str("- ");
        prompt.push_str(file);
        prompt.push('\n');
    }
    prompt.push_str("\nCALLSIEVE_AGENT_CONTEXT_JSON:\n");
    prompt.push_str(&serde_json::to_string_pretty(&compact_context)?);
    prompt.push_str("\n\nReturn JSON only.\n");

    Ok(PilotPromptPlan {
        command,
        files_read,
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
        .filter_map(|file| file.get("file").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
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
        .get("symbols")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(6)
        .map(|symbol| {
            serde_json::json!({
                "name": symbol.get("name").cloned().unwrap_or(serde_json::Value::Null),
                "kind": symbol.get("kind").cloned().unwrap_or(serde_json::Value::Null),
                "lines": symbol.get("lines").cloned().unwrap_or(serde_json::Value::Null),
                "signature": symbol.get("signature").cloned().unwrap_or(serde_json::Value::Null)
            })
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
        .get("why")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .take(4)
        .cloned()
        .collect::<Vec<_>>();
    serde_json::json!({
        "rank": file.get("rank").cloned().unwrap_or(serde_json::Value::Null),
        "score": file.get("score").cloned().unwrap_or(serde_json::Value::Null),
        "file": file.get("file").cloned().unwrap_or(serde_json::Value::Null),
        "risk": file
            .get("blast_radius")
            .and_then(|blast_radius| blast_radius.get("risk"))
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "symbols": symbols,
        "snippets": snippets,
        "related_tests": related_tests,
        "why": why
    })
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
    let proof_manifest: query::BenchmarkReportManifest =
        serde_json::from_value(proof_manifest.clone())?;
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
        "cursor" => AgentClient::Cursor,
        "cline" => AgentClient::Cline,
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
    callsieve.files_read.sort();
    callsieve.files_read.dedup();

    let baseline_value = session_metrics_value(&baseline);
    let callsieve_value = session_metrics_value(&callsieve);
    let expected_files = json_string_array(value.get("expected_files"));
    let callsieve_files: std::collections::BTreeSet<&str> =
        callsieve.files_read.iter().map(String::as_str).collect();
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
    if classification == "grep" || is_grep_command_local(command) {
        metrics.grep_commands += 1;
    }
    if !files_read.is_empty() {
        metrics.file_reads += files_read.len();
        metrics.files_read.extend(files_read);
    } else if classification == "file_read" || is_file_read_command_local(command) {
        metrics.file_reads += 1;
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
        "files_read": metrics.files_read.clone()
    })
}

fn empty_session_metrics() -> serde_json::Value {
    serde_json::json!({
        "grep_commands": 0,
        "file_reads": 0,
        "tokens": 0,
        "commands": [],
        "files_read": []
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
    } else if is_grep_command_local(command) {
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

fn is_file_read_command_local(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    matches!(
        first,
        "cat" | "less" | "more" | "head" | "tail" | "sed" | "nl" | "bat" | "type" | "get-content"
    ) || lower.contains(" get-content ")
        || lower.starts_with("read_file")
        || lower.contains(" read_file")
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

    Ok(SetupAgentOutput {
        command: "setup-agent",
        client: agent_client_name(client).to_string(),
        root: root_label(root),
        files: written,
        first_required_command,
        policy: "Call callsieve_context before broad grep, rg, repository search, or repeated file reads.",
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

fn install_hook(
    root: &Path,
    client: AgentClient,
    strict: bool,
    force: bool,
    lsp: bool,
) -> Result<HookInstallOutput> {
    let index = build_index_output(root, lsp)?;
    let setup = setup_agent(client, root, force)?;
    let first_required_command = setup.first_required_command.clone();
    let shim = install_shim(root, force, strict)?;
    let launchers = write_hook_launchers(root, &first_required_command, force)?;

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
        launchers,
        first_required_command,
        policy: "repo-local hook only; launchers prepend .callsieve/bin for that process and do not mutate global PATH",
    })
}

fn hook_doctor(root: &Path) -> HookDoctorOutput {
    let launchers = hook_launcher_paths(root);
    let launchers_installed = launchers.iter().all(|path| path.is_file());
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
    let status = if checks.iter().all(|check| check.status == "pass") {
        "pass"
    } else {
        "fail"
    }
    .to_string();

    HookDoctorOutput {
        command: "hook doctor",
        status,
        root: root_label(root),
        checks,
        path_instruction: shim.path_instruction.clone(),
        shim,
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
    output.command = "hook uninstall";
    Ok(output)
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
                    "Run `{daemon_command}` at project open, then call `callsieve agent-context <repo> \"<task>\"` before broad grep or repeated file reads.\n"
                ),
                force,
                &mut files,
            )?;
            write_project_file(
                root,
                &root.join(".callsieve/editor-hook.json"),
                &serde_json::to_string_pretty(&serde_json::json!({
                    "daemon_command": daemon_command.clone(),
                    "policy": "callsieve_context before broad grep or repeated file reads"
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
    "CallSieve policy: call callsieve_context with the repository path and task before broad grep, rg, repository-wide search, or repeated file reads. Read read_first files first; grep only if the context packet is insufficient.\n"
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

    let shim = shim_doctor(root);
    let shim_files = shim_files_installed(root);
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
            (root.join(".codex/CALLSIEVE.md"), policy.clone()),
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
            (root.join("CLAUDE.md"), policy.clone()),
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
            (root.join(".cursor/rules/callsieve.mdc"), policy.clone()),
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
            (root.join(".clinerules/callsieve.md"), policy.clone()),
        ],
        AgentClient::Roo => vec![
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
        AgentClient::Cursor => {
            "Use the Cursor MCP config and this rule file before repository search."
        }
        AgentClient::Cline => "Use the Cline MCP server and this rule before search tools.",
        AgentClient::Roo => "Use the Roo MCP server and this rule before search tools.",
        AgentClient::Generic => "Use `callsieve mcp` for MCP clients when available.",
    };

    format!(
        "CallSieve policy for {client_name}:\n\
1. First command for every coding task: `{first_required_command}`.\n\
2. {mcp_hint}\n\
3. Read the returned `read_first` files before broad grep, rg, repository-wide search, or repeated file reads.\n\
4. grep only if the context packet is insufficient, and preserve why it was insufficient in the task notes or trace.\n\
5. For audited sessions, run `callsieve enforce <repo> --client {client_name} --trace <trace.json> --strict`.\n"
    )
}

fn callsieve_executable_display() -> String {
    env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("callsieve"))
        .display()
        .to_string()
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
        AgentClient::Cursor => "cursor",
        AgentClient::Cline => "cline",
        AgentClient::Roo => "roo",
        AgentClient::Generic => "generic",
    }
}

fn daemon_index_snapshot(root: &Path) -> DaemonIndexSnapshot {
    let Some(index) = store::json_store::load_index(root).ok() else {
        return DaemonIndexSnapshot {
            last_indexed_at: 0,
            index_generation: 0,
            stale_files: 0,
            last_error: None,
        };
    };
    let stale_files = serde_json::to_value(query::index_status(root, Some(&index)))
        .ok()
        .and_then(|value| value.get("stale_files").and_then(serde_json::Value::as_u64))
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();

    DaemonIndexSnapshot {
        last_indexed_at: index.metadata.indexed_at,
        index_generation: index.metadata.index_generation,
        stale_files,
        last_error: index.metadata.last_error,
    }
}

fn run_daemon(
    root: &Path,
    lsp: bool,
    interval_ms: u64,
    foreground: bool,
    once: bool,
) -> Result<DaemonOutput> {
    let snapshot = daemon_index_snapshot(root);
    if !foreground && !once {
        clear_daemon_stop(root)?;
        let pid = if env::var_os("CALLSIEVE_TEST_BACKGROUND_NO_SPAWN").is_some() {
            0
        } else {
            let exe = env::current_exe().context("failed to resolve current executable")?;
            let mut command = ProcessCommand::new(exe);
            command
                .arg("daemon")
                .arg(root)
                .arg("--foreground")
                .arg("--interval-ms")
                .arg(interval_ms.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if lsp {
                command.arg("--lsp");
            }
            #[cfg(windows)]
            {
                const DETACHED_PROCESS: u32 = 0x0000_0008;
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
                command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            }
            command
                .spawn()
                .context("failed to spawn callsieve daemon")?
                .id()
        };
        let state = DaemonState {
            status: "starting".to_string(),
            root: root_label(root),
            mode: "background".to_string(),
            pid,
            lsp,
            interval_ms,
            started_at: now_unix_seconds(),
            last_indexed_at: snapshot.last_indexed_at,
            last_change_at: snapshot.last_indexed_at,
            index_generation: snapshot.index_generation,
            stale_files: snapshot.stale_files,
            last_error: snapshot.last_error,
        };
        save_daemon_state(root, &state)?;
        return Ok(DaemonOutput {
            command: "daemon",
            state,
        });
    }

    clear_daemon_stop(root)?;
    let mut state = DaemonState {
        status: if once { "indexing_once" } else { "running" }.to_string(),
        root: root_label(root),
        mode: if once { "once" } else { "foreground" }.to_string(),
        pid: std::process::id(),
        lsp,
        interval_ms,
        started_at: now_unix_seconds(),
        last_indexed_at: snapshot.last_indexed_at,
        last_change_at: snapshot.last_indexed_at,
        index_generation: snapshot.index_generation,
        stale_files: snapshot.stale_files,
        last_error: snapshot.last_error,
    };
    save_daemon_state(root, &state)?;

    loop {
        match refresh_watch_index(root, "daemon", &state.mode, lsp) {
            Ok(output) => {
                let status_value = serde_json::to_value(&output.status)?;
                state.status = if once { "indexed_once" } else { "running" }.to_string();
                state.last_indexed_at = now_unix_seconds();
                state.last_change_at = state.last_indexed_at;
                state.index_generation = status_value
                    .get("index_generation")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default();
                state.stale_files = status_value
                    .get("stale_files")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                state.last_error = None;
            }
            Err(error) => {
                state.status = "error".to_string();
                state.last_error = Some(error.to_string());
            }
        }
        save_daemon_state(root, &state)?;

        if once || daemon_stop_path(root).is_file() {
            state.status = if once { "indexed_once" } else { "stopped" }.to_string();
            save_daemon_state(root, &state)?;
            break;
        }

        thread::sleep(Duration::from_millis(interval_ms));
    }

    Ok(DaemonOutput {
        command: "daemon",
        state,
    })
}

fn callsieve_dir(root: &Path) -> PathBuf {
    root.join(store::json_store::INDEX_DIR)
}

fn daemon_state_path(root: &Path) -> PathBuf {
    callsieve_dir(root).join("daemon.json")
}

fn daemon_stop_path(root: &Path) -> PathBuf {
    callsieve_dir(root).join("daemon.stop")
}

fn save_daemon_state(root: &Path, state: &DaemonState) -> Result<()> {
    let dir = callsieve_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(daemon_state_path(root), serde_json::to_vec_pretty(state)?)
        .with_context(|| format!("failed to write daemon state for {}", root.display()))
}

fn load_daemon_state(root: &Path) -> Option<DaemonState> {
    fs::read(daemon_state_path(root))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
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

fn write_daemon_stop(root: &Path) -> Result<()> {
    let dir = callsieve_dir(root);
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    fs::write(daemon_stop_path(root), b"stop")
        .with_context(|| format!("failed to write daemon stop marker for {}", root.display()))
}

fn clear_daemon_stop(root: &Path) -> Result<()> {
    let path = daemon_stop_path(root);
    if path.is_file() {
        fs::remove_file(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
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
        .filter_map(|file| file.get("file").and_then(serde_json::Value::as_str))
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
        let index = store::json_store::load_index(root)?;
        Some(query::build_context_with_options(
            root, &index, pattern, 8, 2, true, false,
        )?)
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
        Cli::try_parse_from(["callsieve", "index", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "index", ".", "--lsp"]).unwrap();
        Cli::try_parse_from(["callsieve", "symbols", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "symbol", ".", "UserService"]).unwrap();
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
        Cli::try_parse_from(["callsieve", "status", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon", ".", "--once"]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon", ".", "--background", "--lsp"]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon-status", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "daemon-stop", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "watch", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "watch", ".", "--lsp"]).unwrap();
        Cli::try_parse_from(["callsieve", "agent-setup", ".", "--client", "codex"]).unwrap();
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
        Cli::try_parse_from([
            "callsieve",
            "codex-bootstrap",
            ".",
            "--model",
            "gpt-5-codex",
        ])
        .unwrap();
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
}
