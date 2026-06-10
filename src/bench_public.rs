//! Public benchmark - Mode A (retrieval-only, offline, deterministic).
//!
//! Workstream 5, smallest shippable slice. For each entry in a manifest, the
//! evaluator:
//!   1. Resolves the repo at `<repos_dir>/<owner>/<repo>` (must already be
//!      cloned and checked out at the pinned `base_commit` - this module never
//!      touches the network).
//!   2. Builds a CallSieve index from that working tree.
//!   3. Calls `agent-context` with the task description.
//!   4. Computes `first_correct_file_rate_at_k` (1.0 if any of the top-K
//!      read-first files is in `ground_truth_files`, else 0.0), records the
//!      top-K file list and selected_files_count.
//!   5. Aggregates across entries and writes a JSON report.
//!
//! No LLM is invoked. `bench-public` makes no network calls. `bench-run` is an
//! explicit opt-in orchestrator that runs one documented `git clone` per
//! distinct repo and then evaluates pinned local checkouts.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "embed")]
use std::cell::RefCell;

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::{indexer, query};

/// Default K used when neither the CLI flag nor the manifest specify one.
pub const DEFAULT_K: usize = 5;

/// Limit passed to `build_context`. We need at least K, but ask for headroom
/// so that ranking ties and the K cap are not artificially conflated.
const CONTEXT_LIMIT: usize = 8;
const SNIPPETS_PER_FILE: usize = 1;

/// Benchmarks pick the embedding model via CALLSIEVE_BENCH_EMBED_MODEL
/// (`code` selects the jina code model) so A/B model runs need no CLI churn.
#[cfg(feature = "embed")]
fn bench_embedder() -> anyhow::Result<crate::query::embed::FastembedEmbedder> {
    if std::env::var("CALLSIEVE_BENCH_EMBED_MODEL").as_deref() == Ok("code") {
        crate::query::embed::FastembedEmbedder::new_code()
    } else {
        crate::query::embed::FastembedEmbedder::new_default()
    }
}

#[derive(Clone, Copy)]
pub struct RunOptions<'a> {
    pub embeddings: bool,
    #[cfg(feature = "embed")]
    pub embedder: Option<&'a dyn query::embed::LocalEmbedder>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl Default for RunOptions<'_> {
    fn default() -> Self {
        Self {
            embeddings: false,
            #[cfg(feature = "embed")]
            embedder: None,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> RunOptions<'a> {
    pub fn embeddings(embeddings: bool) -> Self {
        Self {
            embeddings,
            #[cfg(feature = "embed")]
            embedder: None,
            _marker: std::marker::PhantomData,
        }
    }
}

/// Parsed manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    /// Optional schema version for forward-compat.
    #[serde(default)]
    pub schema_version: Option<u32>,
    /// Free-form description recorded in the output report for traceability.
    #[serde(default)]
    pub description: Option<String>,
    /// Mode tag ("A" today; Mode B is a future workstream).
    #[serde(default)]
    pub mode: Option<String>,
    /// Default K when the CLI flag and per-issue override are absent.
    #[serde(default)]
    pub default_k: Option<usize>,
    /// Issues to evaluate.
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    /// Stable identifier (e.g. "psf__requests-1963").
    pub id: String,
    /// `<owner>/<name>` slug.
    pub repo: String,
    /// Pinned base commit SHA. Recorded for traceability; this evaluator does
    /// not verify the working tree's HEAD - the runner is responsible for the
    /// checkout.
    pub base_commit: String,
    /// Natural-language task description (passed verbatim to agent-context).
    pub task: String,
    /// Paths the resolving PR modifies, repo-relative, forward-slash.
    pub ground_truth_files: Vec<String>,
    /// Resolving PR number, traceability only.
    #[serde(default)]
    pub resolving_pr: Option<u64>,
}

impl Manifest {
    /// Parse a manifest from a JSON string.
    pub fn from_str(json: &str) -> Result<Self> {
        let manifest: Manifest = serde_json::from_str(json)
            .context("failed to parse benchmarks/public manifest JSON")?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        if self.issues.is_empty() {
            bail!("manifest must contain at least one issue");
        }
        let mut seen = BTreeSet::new();
        for issue in &self.issues {
            if issue.id.is_empty() {
                bail!("issue is missing `id`");
            }
            if !seen.insert(issue.id.clone()) {
                bail!("duplicate issue id `{}` in manifest", issue.id);
            }
            if issue.repo.split('/').count() != 2 {
                bail!(
                    "issue `{}` has invalid `repo` `{}` (expected `<owner>/<name>`)",
                    issue.id,
                    issue.repo
                );
            }
            if issue.base_commit.is_empty() {
                bail!("issue `{}` is missing `base_commit`", issue.id);
            }
            if issue.task.trim().is_empty() {
                bail!("issue `{}` has empty `task`", issue.id);
            }
            if issue.ground_truth_files.is_empty() {
                bail!("issue `{}` has empty `ground_truth_files`", issue.id);
            }
        }
        Ok(())
    }
}

/// Per-issue Mode A result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueResult {
    pub id: String,
    pub repo: String,
    pub base_commit: String,
    pub task: String,
    pub ground_truth_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolving_pr: Option<u64>,
    /// K used to score this issue.
    pub k: usize,
    /// The top-K read-first file paths returned by agent-context.
    pub top_k_files: Vec<String>,
    /// `len(read_first)` from the agent-context packet (before slicing to K).
    pub selected_files_count: usize,
    /// 1.0 if any of `top_k_files` is in `ground_truth_files`, else 0.0.
    pub first_correct_file_rate_at_k: f64,
    /// Files in both `top_k_files` and `ground_truth_files`. Empty on a miss.
    pub matched_files: Vec<String>,
    /// Non-fatal warning (e.g. repo missing); when set the issue is skipped
    /// and contributes to neither numerator nor denominator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

/// Aggregated Mode A report. Serialized to disk.
#[derive(Debug, Clone, Serialize)]
pub struct ModeAReport {
    pub mode: &'static str,
    pub schema_version: u32,
    pub generated_at_unix: u64,
    pub generated_at_iso_date: String,
    pub manifest: ManifestSummary,
    pub k: usize,
    pub aggregate: Aggregate,
    pub issues: Vec<IssueResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareReport {
    pub mode: &'static str,
    pub schema_version: u32,
    pub generated_at_unix: u64,
    pub generated_at_iso_date: String,
    pub manifest: ManifestSummary,
    pub k: usize,
    pub aggregate: CompareAggregate,
    pub issues: Vec<CompareIssueResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareIssueResult {
    pub id: String,
    pub repo: String,
    pub base_commit: String,
    pub task: String,
    pub ground_truth_files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolving_pr: Option<u64>,
    pub k: usize,
    /// "identifier" or "natural_language" (from `query::classify::query_kind`).
    /// Lets us report the hybrid lift on the NL subset separately from the
    /// identifier guardrail. Defaulted for back-compat with older result JSON.
    #[serde(default)]
    pub query_kind: String,
    /// Deterministic ripgrep baseline arm. `None` when `rg` is unavailable or
    /// the issue is skipped. Gives a non-self comparison alongside lexical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grep: Option<CompareArmResult>,
    pub lexical: CompareArmResult,
    pub hybrid: CompareArmResult,
    pub delta: f64,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareArmResult {
    pub top_k_files: Vec<String>,
    pub selected_files_count: usize,
    pub first_correct_file_rate_at_k: f64,
    pub matched_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareAggregate {
    pub evaluated: usize,
    pub skipped: usize,
    pub total: usize,
    pub lexical_first_correct_file_rate_at_k: f64,
    pub hybrid_first_correct_file_rate_at_k: f64,
    pub delta: f64,
    pub wins: usize,
    pub losses: usize,
    pub ties: usize,
    /// Per-query-type breakdown. The hybrid bet is "lift natural-language tasks
    /// while holding identifier tasks flat", so the headline lives in
    /// `natural_language` and the guardrail in `identifier`.
    pub natural_language: QueryKindAggregate,
    pub identifier: QueryKindAggregate,
    /// Deterministic ripgrep baseline over issues that produced a grep arm.
    /// `None` when `rg` was unavailable for the whole run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grep_first_correct_file_rate_at_k: Option<f64>,
    /// `hybrid_rate - grep_rate` over the same grep-evaluated issues.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hybrid_minus_grep: Option<f64>,
    /// Issues that produced a grep arm (denominator for the grep rate).
    pub grep_evaluated: usize,
}

/// Lexical-vs-hybrid rates and delta restricted to one `QueryKind`.
#[derive(Debug, Clone, Serialize)]
pub struct QueryKindAggregate {
    pub evaluated: usize,
    pub lexical_first_correct_file_rate_at_k: f64,
    pub hybrid_first_correct_file_rate_at_k: f64,
    pub delta: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestSummary {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub issue_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Aggregate {
    /// Issues evaluated (non-skipped).
    pub evaluated: usize,
    /// Issues skipped (e.g. repo not cloned).
    pub skipped: usize,
    /// Total issues attempted.
    pub total: usize,
    /// Mean of `first_correct_file_rate_at_k` over evaluated issues. NaN-safe:
    /// 0.0 when nothing evaluated.
    pub first_correct_file_rate_at_k: f64,
    /// Convenience: numerator and denominator behind the rate above.
    pub hits: usize,
}

/// Run the full Mode A evaluation.
#[allow(dead_code)]
pub fn run(
    manifest_path: &Path,
    repos_dir: &Path,
    k_override: Option<usize>,
) -> Result<ModeAReport> {
    run_with_options(manifest_path, repos_dir, k_override, RunOptions::default())
}

pub fn run_with_options(
    manifest_path: &Path,
    repos_dir: &Path,
    k_override: Option<usize>,
    options: RunOptions<'_>,
) -> Result<ModeAReport> {
    let manifest_json = fs::read_to_string(manifest_path).with_context(|| {
        format!(
            "failed to read benchmark manifest: {}",
            manifest_path.display()
        )
    })?;
    let manifest = Manifest::from_str(&manifest_json)?;
    let k = k_override
        .or(manifest.default_k)
        .unwrap_or(DEFAULT_K)
        .max(1);

    let mut issues: Vec<IssueResult> = Vec::with_capacity(manifest.issues.len());
    for issue in &manifest.issues {
        issues.push(evaluate_issue(issue, repos_dir, k, options)?);
    }
    let aggregate = aggregate(&issues);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let iso_date = unix_seconds_to_iso_date(now);

    Ok(ModeAReport {
        mode: "A",
        schema_version: 1,
        generated_at_unix: now,
        generated_at_iso_date: iso_date,
        manifest: ManifestSummary {
            path: manifest_path.display().to_string(),
            schema_version: manifest.schema_version,
            mode: manifest.mode.clone(),
            description: manifest.description.clone(),
            issue_count: manifest.issues.len(),
        },
        k,
        aggregate,
        issues,
    })
}

#[cfg(feature = "embed")]
pub fn run_compare(
    manifest_path: &Path,
    repos_dir: &Path,
    k_override: Option<usize>,
) -> Result<CompareReport> {
    let embedder = bench_embedder()?;
    let embedder = MemoizingEmbedder::new(&embedder);
    run_compare_with_embedder(manifest_path, repos_dir, k_override, &embedder)
}

#[cfg(not(feature = "embed"))]
pub fn run_compare(
    _manifest_path: &Path,
    _repos_dir: &Path,
    _k_override: Option<usize>,
) -> Result<CompareReport> {
    bail!("--compare requires building with --features embed");
}

#[cfg(feature = "embed")]
pub fn run_compare_with_embedder(
    manifest_path: &Path,
    repos_dir: &Path,
    k_override: Option<usize>,
    embedder: &dyn query::embed::LocalEmbedder,
) -> Result<CompareReport> {
    let manifest_json = fs::read_to_string(manifest_path).with_context(|| {
        format!(
            "failed to read benchmark manifest: {}",
            manifest_path.display()
        )
    })?;
    let manifest = Manifest::from_str(&manifest_json)?;
    let k = k_override
        .or(manifest.default_k)
        .unwrap_or(DEFAULT_K)
        .max(1);

    let mut issues = Vec::with_capacity(manifest.issues.len());
    for issue in &manifest.issues {
        issues.push(evaluate_issue_compare(issue, repos_dir, k, embedder)?);
    }
    let aggregate = aggregate_compare(&issues);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let iso_date = unix_seconds_to_iso_date(now);

    Ok(CompareReport {
        mode: "A/B",
        schema_version: 1,
        generated_at_unix: now,
        generated_at_iso_date: iso_date,
        manifest: ManifestSummary {
            path: manifest_path.display().to_string(),
            schema_version: manifest.schema_version,
            mode: manifest.mode.clone(),
            description: manifest.description.clone(),
            issue_count: manifest.issues.len(),
        },
        k,
        aggregate,
        issues,
    })
}

/// Clone/checkout runner for Mode A. Unlike `run_with_options`, this prepares
/// each pinned commit before evaluating it.
pub fn run_bench(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
) -> Result<ModeAReport> {
    run_bench_with_options(
        manifest_path,
        workdir,
        k_override,
        limit,
        RunOptions::default(),
    )
}

pub fn run_bench_with_options(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
    options: RunOptions<'_>,
) -> Result<ModeAReport> {
    run_bench_with_resume_output(manifest_path, workdir, k_override, limit, options, None)
}

pub fn run_bench_with_resume(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
    options: RunOptions<'_>,
    out_path: &Path,
) -> Result<ModeAReport> {
    run_bench_with_resume_output(
        manifest_path,
        workdir,
        k_override,
        limit,
        options,
        Some(out_path),
    )
}

fn run_bench_with_resume_output(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
    options: RunOptions<'_>,
    resume_out_path: Option<&Path>,
) -> Result<ModeAReport> {
    let manifest = load_manifest(manifest_path)?;
    let k = benchmark_k(&manifest, k_override);
    let selected_count = selected_issue_count(&manifest, limit);
    let selected = &manifest.issues[..selected_count];
    let mut previous = load_mode_a_resume(resume_out_path, k)?;
    let pending: Vec<Issue> = selected
        .iter()
        .filter(|issue| !mode_a_resume_matches(previous.get(issue.id.as_str()), issue, k))
        .cloned()
        .collect();

    prepare_bench_repos(&pending, workdir)?;

    let mut issues = Vec::with_capacity(selected.len());
    for (offset, issue) in selected.iter().enumerate() {
        if let Some(result) = take_mode_a_resume(&mut previous, issue, k) {
            eprintln!(
                "bench-run: reusing {}/{} {} at {}",
                offset + 1,
                selected_count,
                issue.id,
                short_commit(&issue.base_commit)
            );
            issues.push(result);
            write_mode_a_resume_report(resume_out_path, manifest_path, &manifest, k, &issues)?;
            continue;
        }
        checkout_issue(workdir, issue)?;
        eprintln!(
            "bench-run: evaluating {}/{} {} at {}",
            offset + 1,
            selected_count,
            issue.id,
            short_commit(&issue.base_commit)
        );
        issues.push(evaluate_issue(issue, workdir, k, options)?);
        write_mode_a_resume_report(resume_out_path, manifest_path, &manifest, k, &issues)?;
    }

    Ok(mode_a_report(manifest_path, &manifest, k, issues))
}

#[cfg(feature = "embed")]
pub fn run_bench_compare(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
) -> Result<CompareReport> {
    let embedder = bench_embedder()?;
    let embedder = MemoizingEmbedder::new(&embedder);
    run_bench_compare_with_embedder(manifest_path, workdir, k_override, limit, &embedder)
}

#[cfg(feature = "embed")]
pub fn run_bench_compare_resume(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
    out_path: &Path,
) -> Result<CompareReport> {
    let embedder = bench_embedder()?;
    let embedder = MemoizingEmbedder::new(&embedder);
    run_bench_compare_with_embedder_resume(
        manifest_path,
        workdir,
        k_override,
        limit,
        out_path,
        &embedder,
    )
}

#[cfg(not(feature = "embed"))]
pub fn run_bench_compare(
    _manifest_path: &Path,
    _workdir: &Path,
    _k_override: Option<usize>,
    _limit: Option<usize>,
) -> Result<CompareReport> {
    bail!("bench-run --compare requires building with --features embed");
}

#[cfg(not(feature = "embed"))]
pub fn run_bench_compare_resume(
    _manifest_path: &Path,
    _workdir: &Path,
    _k_override: Option<usize>,
    _limit: Option<usize>,
    _out_path: &Path,
) -> Result<CompareReport> {
    bail!("bench-run --compare requires building with --features embed");
}

#[cfg(feature = "embed")]
pub fn run_bench_compare_with_embedder(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
    embedder: &dyn query::embed::LocalEmbedder,
) -> Result<CompareReport> {
    run_bench_compare_with_embedder_resume_output(
        manifest_path,
        workdir,
        k_override,
        limit,
        None,
        embedder,
    )
}

#[cfg(feature = "embed")]
pub fn run_bench_compare_with_embedder_resume(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
    out_path: &Path,
    embedder: &dyn query::embed::LocalEmbedder,
) -> Result<CompareReport> {
    run_bench_compare_with_embedder_resume_output(
        manifest_path,
        workdir,
        k_override,
        limit,
        Some(out_path),
        embedder,
    )
}

#[cfg(feature = "embed")]
fn run_bench_compare_with_embedder_resume_output(
    manifest_path: &Path,
    workdir: &Path,
    k_override: Option<usize>,
    limit: Option<usize>,
    resume_out_path: Option<&Path>,
    embedder: &dyn query::embed::LocalEmbedder,
) -> Result<CompareReport> {
    let manifest = load_manifest(manifest_path)?;
    let k = benchmark_k(&manifest, k_override);
    let selected_count = selected_issue_count(&manifest, limit);
    let selected = &manifest.issues[..selected_count];
    let mut previous = load_compare_resume(resume_out_path, k)?;
    let pending: Vec<Issue> = selected
        .iter()
        .filter(|issue| !compare_resume_matches(previous.get(issue.id.as_str()), issue, k))
        .cloned()
        .collect();

    prepare_bench_repos(&pending, workdir)?;

    let mut issues = Vec::with_capacity(selected.len());
    for (offset, issue) in selected.iter().enumerate() {
        if let Some(result) = take_compare_resume(&mut previous, issue, k) {
            eprintln!(
                "bench-run: reusing {}/{} {} at {}",
                offset + 1,
                selected_count,
                issue.id,
                short_commit(&issue.base_commit)
            );
            issues.push(result);
            write_compare_resume_report(resume_out_path, manifest_path, &manifest, k, &issues)?;
            continue;
        }
        checkout_issue(workdir, issue)?;
        eprintln!(
            "bench-run: comparing {}/{} {} at {}",
            offset + 1,
            selected_count,
            issue.id,
            short_commit(&issue.base_commit)
        );
        issues.push(evaluate_issue_compare(issue, workdir, k, embedder)?);
        write_compare_resume_report(resume_out_path, manifest_path, &manifest, k, &issues)?;
    }

    Ok(compare_report(manifest_path, &manifest, k, issues))
}

#[cfg(feature = "embed")]
struct MemoizingEmbedder<'a> {
    inner: &'a dyn query::embed::LocalEmbedder,
    cache: RefCell<BTreeMap<String, Vec<f32>>>,
}

#[cfg(feature = "embed")]
impl<'a> MemoizingEmbedder<'a> {
    fn new(inner: &'a dyn query::embed::LocalEmbedder) -> Self {
        Self {
            inner,
            cache: RefCell::new(BTreeMap::new()),
        }
    }
}

#[cfg(feature = "embed")]
impl query::embed::LocalEmbedder for MemoizingEmbedder<'_> {
    fn id(&self) -> query::embed::EmbedderId {
        self.inner.id()
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let mut output = vec![None; texts.len()];
        let mut missing_indices = Vec::new();
        let mut missing_texts = Vec::new();

        {
            let cache = self.cache.borrow();
            for (index, text) in texts.iter().enumerate() {
                if let Some(vector) = cache.get(*text) {
                    output[index] = Some(vector.clone());
                } else {
                    missing_indices.push(index);
                    missing_texts.push(*text);
                }
            }
        }

        if !missing_texts.is_empty() {
            let embedded = self.inner.embed(&missing_texts)?;
            if embedded.len() != missing_texts.len() {
                bail!(
                    "embedder returned {} vectors for {} inputs",
                    embedded.len(),
                    missing_texts.len()
                );
            }
            let mut cache = self.cache.borrow_mut();
            for ((index, text), vector) in
                missing_indices.into_iter().zip(missing_texts).zip(embedded)
            {
                cache.insert(text.to_string(), vector.clone());
                output[index] = Some(vector);
            }
        }

        output
            .into_iter()
            .map(|vector| vector.ok_or_else(|| anyhow!("internal: missing embedding output")))
            .collect()
    }
}

fn load_manifest(manifest_path: &Path) -> Result<Manifest> {
    let manifest_json = fs::read_to_string(manifest_path).with_context(|| {
        format!(
            "failed to read benchmark manifest: {}",
            manifest_path.display()
        )
    })?;
    Manifest::from_str(&manifest_json)
}

fn benchmark_k(manifest: &Manifest, k_override: Option<usize>) -> usize {
    k_override
        .or(manifest.default_k)
        .unwrap_or(DEFAULT_K)
        .max(1)
}

fn selected_issue_count(manifest: &Manifest, limit: Option<usize>) -> usize {
    limit
        .unwrap_or(manifest.issues.len())
        .min(manifest.issues.len())
}

fn manifest_summary(manifest_path: &Path, manifest: &Manifest) -> ManifestSummary {
    ManifestSummary {
        path: manifest_path.display().to_string(),
        schema_version: manifest.schema_version,
        mode: manifest.mode.clone(),
        description: manifest.description.clone(),
        issue_count: manifest.issues.len(),
    }
}

fn report_clock() -> (u64, String) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (now, unix_seconds_to_iso_date(now))
}

#[derive(Debug, Deserialize)]
struct StoredModeAReport {
    mode: String,
    k: usize,
    issues: Vec<IssueResult>,
}

#[cfg(feature = "embed")]
#[derive(Debug, Deserialize)]
struct StoredCompareReport {
    mode: String,
    k: usize,
    issues: Vec<CompareIssueResult>,
}

fn mode_a_report(
    manifest_path: &Path,
    manifest: &Manifest,
    k: usize,
    issues: Vec<IssueResult>,
) -> ModeAReport {
    let aggregate = aggregate(&issues);
    let (now, iso_date) = report_clock();
    ModeAReport {
        mode: "A",
        schema_version: 1,
        generated_at_unix: now,
        generated_at_iso_date: iso_date,
        manifest: manifest_summary(manifest_path, manifest),
        k,
        aggregate,
        issues,
    }
}

#[cfg(feature = "embed")]
fn compare_report(
    manifest_path: &Path,
    manifest: &Manifest,
    k: usize,
    issues: Vec<CompareIssueResult>,
) -> CompareReport {
    let aggregate = aggregate_compare(&issues);
    let (now, iso_date) = report_clock();
    CompareReport {
        mode: "A/B",
        schema_version: 1,
        generated_at_unix: now,
        generated_at_iso_date: iso_date,
        manifest: manifest_summary(manifest_path, manifest),
        k,
        aggregate,
        issues,
    }
}

fn load_mode_a_resume(
    resume_out_path: Option<&Path>,
    k: usize,
) -> Result<BTreeMap<String, IssueResult>> {
    let Some(out_path) = resume_out_path else {
        return Ok(BTreeMap::new());
    };
    if !out_path.is_file() {
        return Ok(BTreeMap::new());
    }
    let json = fs::read_to_string(out_path)
        .with_context(|| format!("failed to read resume report {}", out_path.display()))?;
    let report: StoredModeAReport = serde_json::from_str(&json)
        .with_context(|| format!("failed to parse resume report {}", out_path.display()))?;
    if report.mode != "A" {
        bail!(
            "bench-run --resume expected mode A report at {}, found `{}`",
            out_path.display(),
            report.mode
        );
    }
    if report.k != k {
        bail!(
            "bench-run --resume expected k {k} at {}, found {}",
            out_path.display(),
            report.k
        );
    }
    let mut by_id = BTreeMap::new();
    for issue in report.issues {
        if by_id.insert(issue.id.clone(), issue).is_some() {
            bail!(
                "bench-run --resume report {} contains duplicate issue ids",
                out_path.display()
            );
        }
    }
    Ok(by_id)
}

#[cfg(feature = "embed")]
fn load_compare_resume(
    resume_out_path: Option<&Path>,
    k: usize,
) -> Result<BTreeMap<String, CompareIssueResult>> {
    let Some(out_path) = resume_out_path else {
        return Ok(BTreeMap::new());
    };
    if !out_path.is_file() {
        return Ok(BTreeMap::new());
    }
    let json = fs::read_to_string(out_path)
        .with_context(|| format!("failed to read resume report {}", out_path.display()))?;
    let report: StoredCompareReport = serde_json::from_str(&json)
        .with_context(|| format!("failed to parse resume report {}", out_path.display()))?;
    if report.mode != "A/B" {
        bail!(
            "bench-run --resume expected mode A/B report at {}, found `{}`",
            out_path.display(),
            report.mode
        );
    }
    if report.k != k {
        bail!(
            "bench-run --resume expected k {k} at {}, found {}",
            out_path.display(),
            report.k
        );
    }
    let mut by_id = BTreeMap::new();
    for issue in report.issues {
        if by_id.insert(issue.id.clone(), issue).is_some() {
            bail!(
                "bench-run --resume report {} contains duplicate issue ids",
                out_path.display()
            );
        }
    }
    Ok(by_id)
}

fn mode_a_resume_matches(result: Option<&IssueResult>, issue: &Issue, k: usize) -> bool {
    let Some(result) = result else {
        return false;
    };
    result.skipped.is_none()
        && result.k == k
        && result.id == issue.id
        && result.repo == issue.repo
        && result.base_commit == issue.base_commit
        && result.task == issue.task
        && result.ground_truth_files == issue.ground_truth_files
}

fn take_mode_a_resume(
    previous: &mut BTreeMap<String, IssueResult>,
    issue: &Issue,
    k: usize,
) -> Option<IssueResult> {
    let result = previous.remove(&issue.id)?;
    if mode_a_resume_matches(Some(&result), issue, k) {
        Some(result)
    } else {
        None
    }
}

#[cfg(feature = "embed")]
fn compare_resume_matches(result: Option<&CompareIssueResult>, issue: &Issue, k: usize) -> bool {
    let Some(result) = result else {
        return false;
    };
    result.skipped.is_none()
        && result.k == k
        && result.id == issue.id
        && result.repo == issue.repo
        && result.base_commit == issue.base_commit
        && result.task == issue.task
        && result.ground_truth_files == issue.ground_truth_files
}

#[cfg(feature = "embed")]
fn take_compare_resume(
    previous: &mut BTreeMap<String, CompareIssueResult>,
    issue: &Issue,
    k: usize,
) -> Option<CompareIssueResult> {
    let result = previous.remove(&issue.id)?;
    if compare_resume_matches(Some(&result), issue, k) {
        Some(result)
    } else {
        None
    }
}

fn write_mode_a_resume_report(
    resume_out_path: Option<&Path>,
    manifest_path: &Path,
    manifest: &Manifest,
    k: usize,
    issues: &[IssueResult],
) -> Result<()> {
    if let Some(out_path) = resume_out_path {
        let report = mode_a_report(manifest_path, manifest, k, issues.to_vec());
        write_report(out_path, &report)?;
    }
    Ok(())
}

#[cfg(feature = "embed")]
fn write_compare_resume_report(
    resume_out_path: Option<&Path>,
    manifest_path: &Path,
    manifest: &Manifest,
    k: usize,
    issues: &[CompareIssueResult],
) -> Result<()> {
    if let Some(out_path) = resume_out_path {
        let report = compare_report(manifest_path, manifest, k, issues.to_vec());
        write_compare_report(out_path, &report)?;
    }
    Ok(())
}

fn prepare_bench_repos(issues: &[Issue], workdir: &Path) -> Result<()> {
    let mut repos = BTreeSet::new();
    for issue in issues {
        repos.insert(issue.repo.as_str());
    }
    for repo in repos {
        ensure_repo_clone(workdir, repo)?;
    }
    Ok(())
}

fn ensure_repo_clone(workdir: &Path, repo: &str) -> Result<PathBuf> {
    let (owner, name) = repo_parts(repo)?;
    let dest = workdir.join(owner).join(name);
    if dest.exists() {
        return Ok(dest);
    }
    let parent = dest
        .parent()
        .ok_or_else(|| anyhow!("invalid clone destination {}", dest.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create benchmark workdir {}", parent.display()))?;

    let url = format!("https://github.com/{owner}/{name}.git");
    eprintln!("bench-run: cloning {repo} into {}", dest.display());
    let mut command = ProcessCommand::new("git");
    command
        .arg("clone")
        .arg("--filter=blob:none")
        .arg(&url)
        .arg(&dest);
    run_command(command, format!("git clone {repo}"))?;
    Ok(dest)
}

fn checkout_issue(workdir: &Path, issue: &Issue) -> Result<PathBuf> {
    let repo_path = repo_path(workdir, &issue.repo)?;
    let mut command = ProcessCommand::new("git");
    command
        .arg("-C")
        .arg(&repo_path)
        .arg("checkout")
        .arg("-f")
        .arg(&issue.base_commit);
    run_command(
        command,
        format!(
            "git checkout {} for {}",
            short_commit(&issue.base_commit),
            issue.id
        ),
    )?;
    Ok(repo_path)
}

fn repo_path(workdir: &Path, repo: &str) -> Result<PathBuf> {
    let (owner, name) = repo_parts(repo)?;
    Ok(workdir.join(owner).join(name))
}

fn repo_parts(repo: &str) -> Result<(&str, &str)> {
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| anyhow!("invalid repo slug `{repo}` (expected `<owner>/<name>`)"))?;
    if !valid_repo_part(owner) || !valid_repo_part(name) {
        bail!("invalid repo slug `{repo}`");
    }
    Ok((owner, name))
}

fn valid_repo_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains("..")
        && part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn run_command(mut command: ProcessCommand, description: String) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!(
        "{description} failed with status {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout.trim(),
        stderr.trim()
    );
}

fn short_commit(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

/// Evaluate a single issue. Indexes the repo at `<repos_dir>/<owner>/<name>`
/// and runs agent-context.
fn evaluate_issue(
    issue: &Issue,
    repos_dir: &Path,
    k: usize,
    options: RunOptions<'_>,
) -> Result<IssueResult> {
    let repo_path = repos_dir.join(&issue.repo);
    if !repo_path.exists() {
        return Ok(skipped_issue(
            issue,
            k,
            format!(
                "repo not found at {}; clone <owner>/<repo> there and check out base_commit before running",
                repo_path.display()
            ),
        ));
    }

    eprintln!(
        "bench-run: indexing {} from {}",
        issue.id,
        repo_path.display()
    );
    let index = indexer::build_index(&repo_path)
        .with_context(|| format!("failed to index {}", repo_path.display()))?;
    eprintln!(
        "bench-run: indexed {} files and {} symbols for {}",
        index.files.len(),
        index.symbols.len(),
        issue.id
    );
    build_embeddings_if_requested(&repo_path, &index, options)?;
    if options.embeddings {
        eprintln!("bench-run: wrote embeddings for {}", issue.id);
    }
    let context = query::build_context_with(
        &repo_path,
        &index,
        &issue.task,
        query::ContextOptions {
            limit: CONTEXT_LIMIT,
            snippets_per_file: SNIPPETS_PER_FILE,
            include_snippets: false,
            why_debug: false,
            hybrid: hybrid_options_from_run_options(options),
            error_frames: &[],
            git_boost: false,
            memory_boost: false,
        },
    )
    .with_context(|| format!("failed to build context for {}", issue.id))?;
    eprintln!("bench-run: scored context for {}", issue.id);
    let read_first = read_first_files(&context)?;
    let selected_files_count = read_first.len();
    let top_k: Vec<String> = read_first.into_iter().take(k).collect();
    let ground: BTreeSet<&str> = issue
        .ground_truth_files
        .iter()
        .map(String::as_str)
        .collect();
    let matched: Vec<String> = top_k
        .iter()
        .filter(|file| ground.contains(file.as_str()))
        .cloned()
        .collect();
    let rate = first_correct_file_rate_at_k(&top_k, &issue.ground_truth_files, k);

    Ok(IssueResult {
        id: issue.id.clone(),
        repo: issue.repo.clone(),
        base_commit: issue.base_commit.clone(),
        task: issue.task.clone(),
        ground_truth_files: issue.ground_truth_files.clone(),
        resolving_pr: issue.resolving_pr,
        k,
        top_k_files: top_k,
        selected_files_count,
        first_correct_file_rate_at_k: rate,
        matched_files: matched,
        skipped: None,
    })
}

#[cfg(feature = "embed")]
fn evaluate_issue_compare(
    issue: &Issue,
    repos_dir: &Path,
    k: usize,
    embedder: &dyn query::embed::LocalEmbedder,
) -> Result<CompareIssueResult> {
    let repo_path = repos_dir.join(&issue.repo);
    if !repo_path.exists() {
        return Ok(skipped_compare_issue(
            issue,
            k,
            format!(
                "repo not found at {}; clone <owner>/<repo> there and check out base_commit before running",
                repo_path.display()
            ),
        ));
    }

    eprintln!(
        "bench-run: indexing {} from {}",
        issue.id,
        repo_path.display()
    );
    let index = indexer::build_index(&repo_path)
        .with_context(|| format!("failed to index {}", repo_path.display()))?;
    eprintln!(
        "bench-run: indexed {} files and {} symbols for {}",
        index.files.len(),
        index.symbols.len(),
        issue.id
    );
    query::embed_build::build_and_write_embeds(&repo_path, &index, embedder, true)?;
    eprintln!("bench-run: wrote embeddings for {}", issue.id);
    let lexical_context = query::build_context(
        &repo_path,
        &index,
        &issue.task,
        CONTEXT_LIMIT,
        SNIPPETS_PER_FILE,
        false,
    )
    .with_context(|| format!("failed to build lexical context for {}", issue.id))?;
    let hybrid_context = query::build_context_with(
        &repo_path,
        &index,
        &issue.task,
        query::ContextOptions {
            limit: CONTEXT_LIMIT,
            snippets_per_file: SNIPPETS_PER_FILE,
            include_snippets: false,
            why_debug: false,
            hybrid: query::HybridOptions::with_embedder(true, embedder),
            error_frames: &[],
            git_boost: false,
            memory_boost: false,
        },
    )
    .with_context(|| format!("failed to build hybrid context for {}", issue.id))?;
    eprintln!(
        "bench-run: scored lexical and hybrid context for {}",
        issue.id
    );

    let lexical = compare_arm_from_context(&lexical_context, issue, k)?;
    let hybrid = compare_arm_from_context(&hybrid_context, issue, k)?;
    let grep = grep_arm(&repo_path, issue, k);
    let delta = hybrid.first_correct_file_rate_at_k - lexical.first_correct_file_rate_at_k;
    let outcome = if delta > 0.0 {
        "win"
    } else if delta < 0.0 {
        "loss"
    } else {
        "tie"
    };

    let query_kind =
        query::classify::query_kind(&issue.task, &query::ranker::query_tokens(&issue.task))
            .as_str()
            .to_string();

    Ok(CompareIssueResult {
        id: issue.id.clone(),
        repo: issue.repo.clone(),
        base_commit: issue.base_commit.clone(),
        task: issue.task.clone(),
        ground_truth_files: issue.ground_truth_files.clone(),
        resolving_pr: issue.resolving_pr,
        k,
        query_kind,
        grep,
        lexical,
        hybrid,
        delta,
        outcome: outcome.to_string(),
        skipped: None,
    })
}

/// Deterministic ripgrep baseline arm: tokenize the task, find files matching
/// each content token, rank by distinct-token match count (path tiebreak), take
/// top-K, and score against ground truth. Returns `None` when `rg` is missing,
/// so the run degrades gracefully to lexical-vs-hybrid only.
#[cfg(feature = "embed")]
fn grep_arm(repo_path: &Path, issue: &Issue, k: usize) -> Option<CompareArmResult> {
    if !ripgrep_available() {
        return None;
    }
    let tokens = query::ranker::query_tokens(&issue.task);
    // distinct-token match count per repo-relative file path.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for token in &tokens {
        let output = ProcessCommand::new("rg")
            .arg("--files-with-matches")
            .arg("--no-messages")
            .arg("-i")
            .arg("--")
            .arg(token)
            .current_dir(repo_path)
            .output()
            .ok()?;
        if !output.status.success() {
            // rg exits 1 when there are no matches; that is not an error here.
            continue;
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let path = line.trim_start_matches("./").to_string();
            if path.is_empty() || !seen.insert(path.clone()) {
                continue;
            }
            *counts.entry(path).or_insert(0) += 1;
        }
    }
    // Rank by match count desc, then path asc for a deterministic order.
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let selected_files_count = ranked.len();
    let top_k: Vec<String> = ranked.into_iter().take(k).map(|(path, _)| path).collect();
    let ground: BTreeSet<&str> = issue
        .ground_truth_files
        .iter()
        .map(String::as_str)
        .collect();
    let matched_files: Vec<String> = top_k
        .iter()
        .filter(|file| ground.contains(file.as_str()))
        .cloned()
        .collect();
    let first_correct_file_rate_at_k =
        first_correct_file_rate_at_k(&top_k, &issue.ground_truth_files, k);
    Some(CompareArmResult {
        top_k_files: top_k,
        selected_files_count,
        first_correct_file_rate_at_k,
        matched_files,
    })
}

#[cfg(feature = "embed")]
fn ripgrep_available() -> bool {
    ProcessCommand::new("rg")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

#[cfg(feature = "embed")]
fn compare_arm_from_context(
    context: &query::ContextOutput,
    issue: &Issue,
    k: usize,
) -> Result<CompareArmResult> {
    let read_first = read_first_files(context)?;
    let selected_files_count = read_first.len();
    let top_k: Vec<String> = read_first.into_iter().take(k).collect();
    let ground: BTreeSet<&str> = issue
        .ground_truth_files
        .iter()
        .map(String::as_str)
        .collect();
    let matched_files: Vec<String> = top_k
        .iter()
        .filter(|file| ground.contains(file.as_str()))
        .cloned()
        .collect();
    let first_correct_file_rate_at_k =
        first_correct_file_rate_at_k(&top_k, &issue.ground_truth_files, k);

    Ok(CompareArmResult {
        top_k_files: top_k,
        selected_files_count,
        first_correct_file_rate_at_k,
        matched_files,
    })
}

#[cfg(feature = "embed")]
fn skipped_compare_issue(issue: &Issue, k: usize, reason: String) -> CompareIssueResult {
    let empty = CompareArmResult {
        top_k_files: Vec::new(),
        selected_files_count: 0,
        first_correct_file_rate_at_k: 0.0,
        matched_files: Vec::new(),
    };
    CompareIssueResult {
        id: issue.id.clone(),
        repo: issue.repo.clone(),
        base_commit: issue.base_commit.clone(),
        task: issue.task.clone(),
        ground_truth_files: issue.ground_truth_files.clone(),
        resolving_pr: issue.resolving_pr,
        k,
        query_kind: query::classify::query_kind(
            &issue.task,
            &query::ranker::query_tokens(&issue.task),
        )
        .as_str()
        .to_string(),
        grep: None,
        lexical: empty.clone(),
        hybrid: empty,
        delta: 0.0,
        outcome: "skipped".to_string(),
        skipped: Some(reason),
    }
}

fn skipped_issue(issue: &Issue, k: usize, reason: String) -> IssueResult {
    IssueResult {
        id: issue.id.clone(),
        repo: issue.repo.clone(),
        base_commit: issue.base_commit.clone(),
        task: issue.task.clone(),
        ground_truth_files: issue.ground_truth_files.clone(),
        resolving_pr: issue.resolving_pr,
        k,
        top_k_files: Vec::new(),
        selected_files_count: 0,
        first_correct_file_rate_at_k: 0.0,
        matched_files: Vec::new(),
        skipped: Some(reason),
    }
}

#[cfg(feature = "embed")]
fn build_embeddings_if_requested(
    repo_path: &Path,
    index: &crate::store::CodeIndex,
    options: RunOptions<'_>,
) -> Result<()> {
    if !options.embeddings {
        return Ok(());
    }
    let owned_embedder;
    let embedder: &dyn query::embed::LocalEmbedder = if let Some(embedder) = options.embedder {
        embedder
    } else {
        owned_embedder = bench_embedder()?;
        &owned_embedder
    };
    query::embed_build::build_and_write_embeds(repo_path, index, embedder, true)?;
    Ok(())
}

#[cfg(not(feature = "embed"))]
fn build_embeddings_if_requested(
    _repo_path: &Path,
    _index: &crate::store::CodeIndex,
    options: RunOptions<'_>,
) -> Result<()> {
    if options.embeddings {
        bail!("--embeddings requires building with --features embed");
    }
    Ok(())
}

#[cfg(feature = "embed")]
fn hybrid_options_from_run_options(options: RunOptions<'_>) -> query::HybridOptions<'_> {
    if let Some(embedder) = options.embedder {
        query::HybridOptions::with_embedder(options.embeddings, embedder)
    } else {
        query::HybridOptions::embeddings(options.embeddings)
    }
}

#[cfg(not(feature = "embed"))]
fn hybrid_options_from_run_options(options: RunOptions<'_>) -> query::HybridOptions<'_> {
    query::HybridOptions::embeddings(options.embeddings)
}

/// Aggregate per-issue results. Skipped issues are excluded from the
/// denominator so a missing clone cannot silently depress the headline number.
pub fn aggregate(issues: &[IssueResult]) -> Aggregate {
    let total = issues.len();
    let skipped = issues.iter().filter(|r| r.skipped.is_some()).count();
    let evaluated = total - skipped;
    let hits = issues
        .iter()
        .filter(|r| r.skipped.is_none() && r.first_correct_file_rate_at_k >= 0.5)
        .count();
    let rate = if evaluated == 0 {
        0.0
    } else {
        hits as f64 / evaluated as f64
    };
    Aggregate {
        evaluated,
        skipped,
        total,
        first_correct_file_rate_at_k: rate,
        hits,
    }
}

#[cfg(feature = "embed")]
pub fn aggregate_compare(issues: &[CompareIssueResult]) -> CompareAggregate {
    let total = issues.len();
    let skipped = issues
        .iter()
        .filter(|issue| issue.skipped.is_some())
        .count();
    let evaluated = total - skipped;
    let mut lexical_hits = 0usize;
    let mut hybrid_hits = 0usize;
    let mut wins = 0usize;
    let mut losses = 0usize;
    let mut ties = 0usize;

    for issue in issues.iter().filter(|issue| issue.skipped.is_none()) {
        if issue.lexical.first_correct_file_rate_at_k >= 0.5 {
            lexical_hits += 1;
        }
        if issue.hybrid.first_correct_file_rate_at_k >= 0.5 {
            hybrid_hits += 1;
        }
        match issue.outcome.as_str() {
            "win" => wins += 1,
            "loss" => losses += 1,
            _ => ties += 1,
        }
    }

    let lexical_rate = if evaluated == 0 {
        0.0
    } else {
        lexical_hits as f64 / evaluated as f64
    };
    let hybrid_rate = if evaluated == 0 {
        0.0
    } else {
        hybrid_hits as f64 / evaluated as f64
    };

    let natural_language = query_kind_aggregate(issues, "natural_language");
    let identifier = query_kind_aggregate(issues, "identifier");

    // Grep baseline over the subset of evaluated issues that produced a grep arm.
    let mut grep_evaluated = 0usize;
    let mut grep_hits = 0usize;
    let mut hybrid_on_grep_hits = 0usize;
    for issue in issues.iter().filter(|issue| issue.skipped.is_none()) {
        if let Some(grep) = &issue.grep {
            grep_evaluated += 1;
            if grep.first_correct_file_rate_at_k >= 0.5 {
                grep_hits += 1;
            }
            if issue.hybrid.first_correct_file_rate_at_k >= 0.5 {
                hybrid_on_grep_hits += 1;
            }
        }
    }
    let (grep_rate, hybrid_minus_grep) = if grep_evaluated == 0 {
        (None, None)
    } else {
        let grep_rate = grep_hits as f64 / grep_evaluated as f64;
        let hybrid_rate_on_grep = hybrid_on_grep_hits as f64 / grep_evaluated as f64;
        (Some(grep_rate), Some(hybrid_rate_on_grep - grep_rate))
    };

    CompareAggregate {
        evaluated,
        skipped,
        total,
        lexical_first_correct_file_rate_at_k: lexical_rate,
        hybrid_first_correct_file_rate_at_k: hybrid_rate,
        delta: hybrid_rate - lexical_rate,
        wins,
        losses,
        ties,
        natural_language,
        identifier,
        grep_first_correct_file_rate_at_k: grep_rate,
        hybrid_minus_grep,
        grep_evaluated,
    }
}

/// Lexical/hybrid rates and delta restricted to evaluated issues whose
/// `query_kind` matches `kind`.
#[cfg(feature = "embed")]
fn query_kind_aggregate(issues: &[CompareIssueResult], kind: &str) -> QueryKindAggregate {
    let mut evaluated = 0usize;
    let mut lexical_hits = 0usize;
    let mut hybrid_hits = 0usize;
    for issue in issues
        .iter()
        .filter(|issue| issue.skipped.is_none() && issue.query_kind == kind)
    {
        evaluated += 1;
        if issue.lexical.first_correct_file_rate_at_k >= 0.5 {
            lexical_hits += 1;
        }
        if issue.hybrid.first_correct_file_rate_at_k >= 0.5 {
            hybrid_hits += 1;
        }
    }
    let (lexical_rate, hybrid_rate) = if evaluated == 0 {
        (0.0, 0.0)
    } else {
        (
            lexical_hits as f64 / evaluated as f64,
            hybrid_hits as f64 / evaluated as f64,
        )
    };
    QueryKindAggregate {
        evaluated,
        lexical_first_correct_file_rate_at_k: lexical_rate,
        hybrid_first_correct_file_rate_at_k: hybrid_rate,
        delta: hybrid_rate - lexical_rate,
    }
}

/// Compute the synthetic recall@K used by the unit tests and by `evaluate_issue`.
/// Returns 1.0 iff at least one of the first `k` items in `read_first` is in
/// `ground_truth_files`, else 0.0. Pulled out of `evaluate_issue` so the unit
/// tests can exercise it on a tiny synthetic input.
pub fn first_correct_file_rate_at_k(
    read_first: &[String],
    ground_truth_files: &[String],
    k: usize,
) -> f64 {
    if k == 0 || ground_truth_files.is_empty() {
        return 0.0;
    }
    let ground: BTreeSet<&str> = ground_truth_files.iter().map(String::as_str).collect();
    let hit = read_first
        .iter()
        .take(k)
        .any(|file| ground.contains(file.as_str()));
    if hit { 1.0 } else { 0.0 }
}

/// Where to write the report. Honors `--out`; otherwise picks
/// `<manifest_dir>/results/mode-a-<YYYY-MM-DD>.json`.
pub fn resolve_output_path(
    manifest_path: &Path,
    out: Option<&Path>,
    report: &ModeAReport,
) -> Result<PathBuf> {
    if let Some(out) = out {
        return Ok(out.to_path_buf());
    }
    let dir = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("results");
    Ok(dir.join(format!("mode-a-{}.json", report.generated_at_iso_date)))
}

pub fn resolve_compare_output_path(
    manifest_path: &Path,
    out: Option<&Path>,
    report: &CompareReport,
) -> Result<PathBuf> {
    if let Some(out) = out {
        return Ok(out.to_path_buf());
    }
    let dir = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("results");
    Ok(dir.join(format!("mode-ab-{}.json", report.generated_at_iso_date)))
}

/// Write the report JSON to disk. Creates the parent directory if missing.
pub fn write_report(out_path: &Path, report: &ModeAReport) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    write_report_json(out_path, &json)
}

pub fn write_compare_report(out_path: &Path, report: &CompareReport) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    write_report_json(out_path, &json)
}

fn write_report_json(out_path: &Path, json: &str) -> Result<()> {
    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create benchmark results directory {}",
                parent.display()
            )
        })?;
    }
    let tmp_path = temporary_report_path(out_path);
    fs::write(&tmp_path, format!("{json}\n")).with_context(|| {
        format!(
            "failed to write temporary benchmark report to {}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, out_path).with_context(|| {
        format!(
            "failed to atomically write benchmark report to {}",
            out_path.display()
        )
    })?;
    Ok(())
}

fn temporary_report_path(out_path: &Path) -> PathBuf {
    let mut tmp_path = out_path.to_path_buf();
    let extension = out_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!("{value}.tmp"))
        .unwrap_or_else(|| "tmp".to_string());
    tmp_path.set_extension(extension);
    tmp_path
}

/// Compact summary printed to stdout after a run. Keeps the actual report on
/// disk and surfaces the headline number plus the output path.
#[derive(Debug, Serialize)]
pub struct SummaryOutput {
    pub command: &'static str,
    pub mode: &'static str,
    pub manifest: String,
    pub report: String,
    pub k: usize,
    pub aggregate: Aggregate,
}

impl SummaryOutput {
    pub fn new(manifest_path: &Path, out_path: &Path, report: &ModeAReport) -> Self {
        Self::new_for_command("bench-public", manifest_path, out_path, report)
    }

    pub fn new_for_command(
        command: &'static str,
        manifest_path: &Path,
        out_path: &Path,
        report: &ModeAReport,
    ) -> Self {
        Self {
            command,
            mode: report.mode,
            manifest: manifest_path.display().to_string(),
            report: out_path.display().to_string(),
            k: report.k,
            aggregate: report.aggregate.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CompareSummaryOutput {
    pub command: &'static str,
    pub mode: &'static str,
    pub manifest: String,
    pub report: String,
    pub k: usize,
    pub aggregate: CompareAggregate,
}

impl CompareSummaryOutput {
    pub fn new(manifest_path: &Path, out_path: &Path, report: &CompareReport) -> Self {
        Self::new_for_command("bench-public", manifest_path, out_path, report)
    }

    pub fn new_for_command(
        command: &'static str,
        manifest_path: &Path,
        out_path: &Path,
        report: &CompareReport,
    ) -> Self {
        Self {
            command,
            mode: report.mode,
            manifest: manifest_path.display().to_string(),
            report: out_path.display().to_string(),
            k: report.k,
            aggregate: report.aggregate.clone(),
        }
    }
}

/// Extract the `read_first[].file` string list from a `ContextOutput`. We go
/// through serde_json so we don't depend on `ContextFile`'s private fields.
fn read_first_files(context: &query::ContextOutput) -> Result<Vec<String>> {
    let value =
        serde_json::to_value(context).context("internal: failed to serialize context output")?;
    let files = value
        .get("read_first")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("internal: context output missing read_first array"))?
        .iter()
        .filter_map(|entry| entry.get("file").and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect();
    Ok(files)
}

/// Tiny no-deps UTC date formatter (YYYY-MM-DD) for the default output filename.
/// We avoid pulling chrono in just for this; the algorithm below is the standard
/// civil-from-days reference (Howard Hinnant). Accurate for any post-epoch date.
fn unix_seconds_to_iso_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    // Shift epoch to 0000-03-01.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_minimal_valid_input() {
        let json = r#"{
            "issues": [
                {
                    "id": "demo-1",
                    "repo": "owner/name",
                    "base_commit": "deadbeef",
                    "task": "Bug X",
                    "ground_truth_files": ["src/foo.py"]
                }
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("parses");
        assert_eq!(manifest.issues.len(), 1);
        assert_eq!(manifest.issues[0].id, "demo-1");
        assert_eq!(manifest.issues[0].repo, "owner/name");
        assert!(manifest.default_k.is_none());
    }

    #[test]
    fn manifest_parses_full_record() {
        let json = r#"{
            "schema_version": 1,
            "description": "Five issues",
            "mode": "A",
            "default_k": 5,
            "issues": [
                {
                    "id": "psf__requests-1963",
                    "repo": "psf/requests",
                    "base_commit": "110048f9837f8441ea536804115e80b69f400277",
                    "task": "redirect handling",
                    "ground_truth_files": ["requests/sessions.py"],
                    "resolving_pr": 1963
                }
            ]
        }"#;
        let manifest = Manifest::from_str(json).expect("parses");
        assert_eq!(manifest.schema_version, Some(1));
        assert_eq!(manifest.mode.as_deref(), Some("A"));
        assert_eq!(manifest.default_k, Some(5));
        assert_eq!(manifest.issues[0].resolving_pr, Some(1963));
    }

    #[test]
    fn manifest_rejects_empty_issue_list() {
        let json = r#"{"issues": []}"#;
        let err = Manifest::from_str(json).expect_err("empty issues should fail");
        assert!(err.to_string().contains("at least one issue"));
    }

    #[test]
    fn manifest_rejects_bad_repo_slug() {
        let json = r#"{
            "issues": [{
                "id": "x",
                "repo": "no-slash",
                "base_commit": "abc",
                "task": "t",
                "ground_truth_files": ["a"]
            }]
        }"#;
        let err = Manifest::from_str(json).expect_err("bad slug should fail");
        assert!(err.to_string().contains("invalid `repo`"));
    }

    #[test]
    fn manifest_rejects_missing_ground_truth() {
        let json = r#"{
            "issues": [{
                "id": "x",
                "repo": "a/b",
                "base_commit": "abc",
                "task": "t",
                "ground_truth_files": []
            }]
        }"#;
        let err = Manifest::from_str(json).expect_err("empty ground truth should fail");
        assert!(err.to_string().contains("ground_truth_files"));
    }

    #[test]
    fn manifest_rejects_duplicate_ids() {
        let json = r#"{
            "issues": [
                {"id": "dup", "repo": "a/b", "base_commit": "1", "task": "t", "ground_truth_files": ["x"]},
                {"id": "dup", "repo": "a/b", "base_commit": "2", "task": "t", "ground_truth_files": ["y"]}
            ]
        }"#;
        let err = Manifest::from_str(json).expect_err("duplicate ids should fail");
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn seed_manifest_on_disk_is_valid() {
        // Smoke test: the committed seed manifest at benchmarks/public/manifest.json
        // must always parse and validate, because third-party reproducers read it
        // verbatim.
        let path = Path::new("benchmarks/public/manifest.json");
        if !path.exists() {
            // Allow the test to be invoked from a workspace where the file has
            // not been generated yet; bench_public's primary contract is the
            // parser above. The CI invocation runs from repo root.
            return;
        }
        let json = std::fs::read_to_string(path).expect("read seed manifest");
        let manifest = Manifest::from_str(&json).expect("seed manifest parses");
        assert_eq!(manifest.issues.len(), 5);
        for issue in &manifest.issues {
            assert_eq!(issue.repo, "psf/requests");
            assert!(!issue.base_commit.is_empty());
            assert!(!issue.ground_truth_files.is_empty());
        }
    }

    #[test]
    fn expanded_manifest_on_disk_is_valid() {
        let path = Path::new("benchmarks/public/manifest-50.json");
        if !path.exists() {
            return;
        }
        let json = std::fs::read_to_string(path).expect("read expanded manifest");
        let manifest = Manifest::from_str(&json).expect("expanded manifest parses");
        assert_eq!(manifest.issues.len(), 50);
        for issue in &manifest.issues {
            assert!(!issue.repo.is_empty());
            assert!(!issue.base_commit.is_empty());
            assert!(!issue.ground_truth_files.is_empty());
        }
    }

    #[cfg(feature = "embed")]
    #[test]
    fn natural_language_manifest_on_disk_is_valid() {
        let path = Path::new("benchmarks/public/manifest-nl.json");
        if !path.exists() {
            return;
        }
        let json = std::fs::read_to_string(path).expect("read natural-language manifest");
        let manifest = Manifest::from_str(&json).expect("natural-language manifest parses");
        assert!(
            manifest.issues.len() >= 30,
            "natural-language manifest should have at least 30 issues"
        );

        let mut natural_language_count = 0usize;
        for issue in &manifest.issues {
            assert!(!issue.repo.is_empty());
            assert!(!issue.base_commit.is_empty());
            assert!(!issue.ground_truth_files.is_empty());
            let tokens = crate::query::ranker::query_tokens(&issue.task);
            let kind = crate::query::classify::query_kind(&issue.task, &tokens);
            if kind == crate::query::classify::QueryKind::NaturalLanguage {
                natural_language_count += 1;
            }
        }

        assert!(
            natural_language_count * 5 >= manifest.issues.len() * 4,
            "expected at least 80% natural-language prompts, got {natural_language_count}/{}",
            manifest.issues.len()
        );
    }

    #[test]
    fn recall_at_k_hits_when_ground_truth_in_top_k() {
        let read_first = vec![
            "src/a.py".to_string(),
            "src/b.py".to_string(),
            "src/c.py".to_string(),
        ];
        let ground = vec!["src/b.py".to_string()];
        assert_eq!(first_correct_file_rate_at_k(&read_first, &ground, 5), 1.0);
    }

    #[test]
    fn recall_at_k_misses_when_ground_truth_below_k() {
        let read_first = vec![
            "src/a.py".to_string(),
            "src/b.py".to_string(),
            "src/c.py".to_string(),
            "src/d.py".to_string(),
            "src/e.py".to_string(),
            "src/f.py".to_string(),
        ];
        let ground = vec!["src/f.py".to_string()];
        // f.py is at position 5 (index 5), which is *outside* top-5 (indices 0..4).
        assert_eq!(first_correct_file_rate_at_k(&read_first, &ground, 5), 0.0);
    }

    #[test]
    fn recall_at_k_handles_empty_inputs() {
        assert_eq!(first_correct_file_rate_at_k(&[], &[], 5), 0.0);
        assert_eq!(
            first_correct_file_rate_at_k(&["a".to_string()], &[], 5),
            0.0
        );
        assert_eq!(
            first_correct_file_rate_at_k(&[], &["a".to_string()], 5),
            0.0
        );
        assert_eq!(
            first_correct_file_rate_at_k(&["a".to_string()], &["a".to_string()], 0),
            0.0
        );
    }

    #[test]
    fn recall_at_k_handles_multi_file_ground_truth() {
        let read_first = vec!["src/a.py".to_string(), "src/b.py".to_string()];
        let ground = vec!["src/x.py".to_string(), "src/a.py".to_string()];
        assert_eq!(first_correct_file_rate_at_k(&read_first, &ground, 2), 1.0);
    }

    fn make_result(id: &str, rate: f64, skipped: bool) -> IssueResult {
        IssueResult {
            id: id.to_string(),
            repo: "owner/name".to_string(),
            base_commit: "abc".to_string(),
            task: "t".to_string(),
            ground_truth_files: vec!["x".to_string()],
            resolving_pr: None,
            k: 5,
            top_k_files: Vec::new(),
            selected_files_count: 0,
            first_correct_file_rate_at_k: rate,
            matched_files: Vec::new(),
            skipped: if skipped {
                Some("repo missing".to_string())
            } else {
                None
            },
        }
    }

    #[test]
    fn aggregate_averages_over_evaluated_only() {
        let results = vec![
            make_result("a", 1.0, false),
            make_result("b", 0.0, false),
            make_result("c", 1.0, false),
            make_result("d", 0.0, true), // skipped
        ];
        let agg = aggregate(&results);
        assert_eq!(agg.total, 4);
        assert_eq!(agg.skipped, 1);
        assert_eq!(agg.evaluated, 3);
        assert_eq!(agg.hits, 2);
        assert!((agg.first_correct_file_rate_at_k - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn aggregate_handles_all_skipped() {
        let results = vec![make_result("a", 0.0, true), make_result("b", 0.0, true)];
        let agg = aggregate(&results);
        assert_eq!(agg.evaluated, 0);
        assert_eq!(agg.skipped, 2);
        assert_eq!(agg.hits, 0);
        assert_eq!(agg.first_correct_file_rate_at_k, 0.0);
    }

    #[test]
    fn aggregate_empty_input_is_zero() {
        let agg = aggregate(&[]);
        assert_eq!(agg.total, 0);
        assert_eq!(agg.evaluated, 0);
        assert_eq!(agg.first_correct_file_rate_at_k, 0.0);
    }

    #[test]
    fn iso_date_formats_known_unix_epoch_values() {
        // 1970-01-01
        assert_eq!(unix_seconds_to_iso_date(0), "1970-01-01");
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(unix_seconds_to_iso_date(946_684_800), "2000-01-01");
        // 2024-02-29 (leap year) 00:00:00 UTC = 1709164800
        assert_eq!(unix_seconds_to_iso_date(1_709_164_800), "2024-02-29");
    }

    #[test]
    fn write_and_resolve_output_path_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("manifest.json");
        std::fs::write(&manifest_path, "{}").unwrap();
        let report = ModeAReport {
            mode: "A",
            schema_version: 1,
            generated_at_unix: 0,
            generated_at_iso_date: "1970-01-01".to_string(),
            manifest: ManifestSummary {
                path: manifest_path.display().to_string(),
                schema_version: None,
                mode: None,
                description: None,
                issue_count: 0,
            },
            k: 5,
            aggregate: aggregate(&[]),
            issues: Vec::new(),
        };
        let out = resolve_output_path(&manifest_path, None, &report).unwrap();
        assert!(
            out.ends_with("results/mode-a-1970-01-01.json"),
            "got {}",
            out.display()
        );
        write_report(&out, &report).unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed["mode"], "A");
        assert_eq!(parsed["k"], 5);
    }

    #[cfg(feature = "embed")]
    struct FakeEmbedder;

    #[cfg(feature = "embed")]
    impl query::embed::LocalEmbedder for FakeEmbedder {
        fn id(&self) -> query::embed::EmbedderId {
            query::embed::EmbedderId::new("fake-bench", "v1")
        }

        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("beta") || text.contains("src/b.rs") {
                        vec![0.0, 1.0]
                    } else {
                        vec![0.0, -1.0]
                    }
                })
                .collect())
        }
    }

    #[cfg(feature = "embed")]
    #[test]
    fn compare_report_records_deterministic_wins_and_losses() {
        let tmp = tempfile::tempdir().unwrap();
        let repos_dir = tmp.path().join("repos");
        let repo = repos_dir.join("owner/name");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/a.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(repo.join("src/b.rs"), "pub fn beta() {}\n").unwrap();
        let manifest_path = tmp.path().join("manifest.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "default_k": 1,
                "issues": [
                    {
                        "id": "win",
                        "repo": "owner/name",
                        "base_commit": "abc",
                        "task": "change alpha beta behavior",
                        "ground_truth_files": ["src/b.rs"]
                    },
                    {
                        "id": "loss",
                        "repo": "owner/name",
                        "base_commit": "abc",
                        "task": "change alpha beta behavior",
                        "ground_truth_files": ["src/a.rs"]
                    }
                ]
            }"#,
        )
        .unwrap();

        let report =
            run_compare_with_embedder(&manifest_path, &repos_dir, Some(1), &FakeEmbedder).unwrap();

        assert_eq!(report.aggregate.evaluated, 2);
        assert_eq!(report.aggregate.wins, 1);
        assert_eq!(report.aggregate.losses, 1);
        assert_eq!(report.aggregate.ties, 0);
    }

    #[cfg(feature = "embed")]
    #[test]
    fn aggregate_compare_splits_by_query_kind_and_scores_grep() {
        fn arm(rate: f64) -> CompareArmResult {
            CompareArmResult {
                top_k_files: Vec::new(),
                selected_files_count: 0,
                first_correct_file_rate_at_k: rate,
                matched_files: Vec::new(),
            }
        }
        fn issue(
            id: &str,
            query_kind: &str,
            lexical: f64,
            hybrid: f64,
            grep: Option<f64>,
        ) -> CompareIssueResult {
            let delta = hybrid - lexical;
            CompareIssueResult {
                id: id.to_string(),
                repo: "owner/name".to_string(),
                base_commit: "abc".to_string(),
                task: "t".to_string(),
                ground_truth_files: Vec::new(),
                resolving_pr: None,
                k: 5,
                query_kind: query_kind.to_string(),
                grep: grep.map(arm),
                lexical: arm(lexical),
                hybrid: arm(hybrid),
                delta,
                outcome: if delta > 0.0 {
                    "win"
                } else if delta < 0.0 {
                    "loss"
                } else {
                    "tie"
                }
                .to_string(),
                skipped: None,
            }
        }

        let issues = vec![
            // NL: hybrid lifts both, grep weaker.
            issue("nl1", "natural_language", 0.0, 1.0, Some(0.0)),
            issue("nl2", "natural_language", 1.0, 1.0, Some(1.0)),
            // identifier: hybrid holds flat (guardrail).
            issue("id1", "identifier", 1.0, 1.0, Some(1.0)),
            // skipped issues never count.
            {
                let mut s = issue("skip", "identifier", 0.0, 0.0, None);
                s.skipped = Some("repo not cloned".to_string());
                s
            },
        ];

        let agg = aggregate_compare(&issues);
        assert_eq!(agg.evaluated, 3);
        assert_eq!(agg.skipped, 1);

        // NL subset: lexical 1/2 = 0.5, hybrid 2/2 = 1.0, delta +0.5.
        assert_eq!(agg.natural_language.evaluated, 2);
        assert_eq!(
            agg.natural_language.lexical_first_correct_file_rate_at_k,
            0.5
        );
        assert_eq!(
            agg.natural_language.hybrid_first_correct_file_rate_at_k,
            1.0
        );
        assert_eq!(agg.natural_language.delta, 0.5);

        // identifier subset: flat at 1.0, delta 0.0 (guardrail).
        assert_eq!(agg.identifier.evaluated, 1);
        assert_eq!(agg.identifier.delta, 0.0);

        // grep aggregate over the 3 evaluated issues with a grep arm.
        assert_eq!(agg.grep_evaluated, 3);
        assert_eq!(agg.grep_first_correct_file_rate_at_k, Some(2.0 / 3.0));
        // hybrid hits all 3 of those, so hybrid - grep = 1.0 - 2/3.
        assert_eq!(agg.hybrid_minus_grep, Some(1.0 - 2.0 / 3.0));
    }

    #[cfg(feature = "embed")]
    fn git(repo: &Path, args: &[&str]) -> String {
        let output = ProcessCommand::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {args:?}: {error}"));
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[cfg(feature = "embed")]
    fn git_commit(repo: &Path, message: &str) -> String {
        git(repo, &["add", "."]);
        git(
            repo,
            &[
                "-c",
                "user.name=CallSieve Tests",
                "-c",
                "user.email=callsieve-tests@example.invalid",
                "commit",
                "-m",
                message,
            ],
        );
        git(repo, &["rev-parse", "HEAD"])
    }

    #[cfg(feature = "embed")]
    fn make_compare_result(
        id: &str,
        repo: &str,
        base_commit: &str,
        task: &str,
        ground_truth_files: Vec<String>,
    ) -> CompareIssueResult {
        let arm = CompareArmResult {
            top_k_files: ground_truth_files.clone(),
            selected_files_count: ground_truth_files.len(),
            first_correct_file_rate_at_k: 1.0,
            matched_files: ground_truth_files.clone(),
        };
        CompareIssueResult {
            id: id.to_string(),
            repo: repo.to_string(),
            base_commit: base_commit.to_string(),
            task: task.to_string(),
            ground_truth_files,
            resolving_pr: None,
            k: 1,
            query_kind: "natural_language".to_string(),
            grep: None,
            lexical: arm.clone(),
            hybrid: arm,
            delta: 0.0,
            outcome: "tie".to_string(),
            skipped: None,
        }
    }

    #[cfg(feature = "embed")]
    fn make_compare_report_for_test(
        manifest_path: &Path,
        issues: Vec<CompareIssueResult>,
        k: usize,
    ) -> CompareReport {
        CompareReport {
            mode: "A/B",
            schema_version: 1,
            generated_at_unix: 0,
            generated_at_iso_date: "1970-01-01".to_string(),
            manifest: ManifestSummary {
                path: manifest_path.display().to_string(),
                schema_version: None,
                mode: None,
                description: None,
                issue_count: 2,
            },
            k,
            aggregate: aggregate_compare(&issues),
            issues,
        }
    }

    #[cfg(feature = "embed")]
    #[test]
    fn bench_run_compare_checks_out_each_issue_and_aggregates() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path().join("work");
        let repo = workdir.join("owner/name");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init"]);
        std::fs::write(repo.join("src/a.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(repo.join("src/b.rs"), "pub fn beta() {}\n").unwrap();
        let first_commit = git_commit(&repo, "initial");
        std::fs::write(repo.join("src/b.rs"), "pub fn beta_delta() {}\n").unwrap();
        let second_commit = git_commit(&repo, "change b");

        let manifest_path = tmp.path().join("manifest.json");
        std::fs::write(
            &manifest_path,
            format!(
                r#"{{
                    "default_k": 1,
                    "issues": [
                        {{
                            "id": "win",
                            "repo": "owner/name",
                            "base_commit": "{first_commit}",
                            "task": "change alpha beta behavior",
                            "ground_truth_files": ["src/b.rs"]
                        }},
                        {{
                            "id": "loss",
                            "repo": "owner/name",
                            "base_commit": "{second_commit}",
                            "task": "change alpha beta behavior",
                            "ground_truth_files": ["src/a.rs"]
                        }}
                    ]
                }}"#
            ),
        )
        .unwrap();

        let report =
            run_bench_compare_with_embedder(&manifest_path, &workdir, Some(1), None, &FakeEmbedder)
                .unwrap();

        assert_eq!(report.aggregate.evaluated, 2);
        assert_eq!(report.aggregate.wins, 1);
        assert_eq!(report.aggregate.losses, 1);
        assert_eq!(report.aggregate.ties, 0);
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), second_commit);
    }

    #[cfg(feature = "embed")]
    #[test]
    fn bench_run_compare_resume_reuses_matching_issue_and_writes_complete_report() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path().join("work");
        let repo = workdir.join("owner/name");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        git(&repo, &["init"]);
        std::fs::write(repo.join("src/a.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(repo.join("src/b.rs"), "pub fn beta() {}\n").unwrap();
        let _first_commit = git_commit(&repo, "initial");
        std::fs::write(repo.join("src/b.rs"), "pub fn beta_delta() {}\n").unwrap();
        let second_commit = git_commit(&repo, "change b");
        let cached_commit = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let manifest_path = tmp.path().join("manifest.json");
        let cached_task = "cached alpha beta behavior";
        let fresh_task = "fresh alpha beta behavior";
        std::fs::write(
            &manifest_path,
            format!(
                r#"{{
                    "default_k": 1,
                    "issues": [
                        {{
                            "id": "cached",
                            "repo": "owner/name",
                            "base_commit": "{cached_commit}",
                            "task": "{cached_task}",
                            "ground_truth_files": ["src/b.rs"]
                        }},
                        {{
                            "id": "fresh",
                            "repo": "owner/name",
                            "base_commit": "{second_commit}",
                            "task": "{fresh_task}",
                            "ground_truth_files": ["src/a.rs"]
                        }}
                    ]
                }}"#
            ),
        )
        .unwrap();

        let out_path = tmp.path().join("compare.json");
        let cached = make_compare_result(
            "cached",
            "owner/name",
            cached_commit,
            cached_task,
            vec!["src/b.rs".to_string()],
        );
        let initial_report = make_compare_report_for_test(&manifest_path, vec![cached], 1);
        write_compare_report(&out_path, &initial_report).unwrap();

        let report = run_bench_compare_with_embedder_resume(
            &manifest_path,
            &workdir,
            Some(1),
            None,
            &out_path,
            &FakeEmbedder,
        )
        .unwrap();

        assert_eq!(report.issues.len(), 2);
        assert_eq!(report.issues[0].id, "cached");
        assert_eq!(report.issues[0].base_commit, cached_commit);
        assert_eq!(report.issues[1].id, "fresh");
        assert_eq!(report.aggregate.total, 2);
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), second_commit);

        let written = std::fs::read_to_string(&out_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(parsed["mode"], "A/B");
        assert_eq!(parsed["issues"].as_array().unwrap().len(), 2);
    }

    #[cfg(feature = "embed")]
    #[test]
    fn bench_run_compare_resume_rejects_k_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest_path = tmp.path().join("manifest.json");
        std::fs::write(
            &manifest_path,
            r#"{
                "default_k": 1,
                "issues": [
                    {
                        "id": "cached",
                        "repo": "owner/name",
                        "base_commit": "abc",
                        "task": "cached alpha beta behavior",
                        "ground_truth_files": ["src/b.rs"]
                    }
                ]
            }"#,
        )
        .unwrap();

        let out_path = tmp.path().join("compare.json");
        let cached = make_compare_result(
            "cached",
            "owner/name",
            "abc",
            "cached alpha beta behavior",
            vec!["src/b.rs".to_string()],
        );
        let initial_report = make_compare_report_for_test(&manifest_path, vec![cached], 2);
        write_compare_report(&out_path, &initial_report).unwrap();

        let err = run_bench_compare_with_embedder_resume(
            &manifest_path,
            &tmp.path().join("work"),
            Some(1),
            None,
            &out_path,
            &FakeEmbedder,
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("expected k 1"),
            "unexpected error: {err:#}"
        );
    }
}
