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
//! No LLM is invoked. No network calls are made.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::{indexer, query};

/// Default K used when neither the CLI flag nor the manifest specify one.
pub const DEFAULT_K: usize = 5;

/// Limit passed to `build_context`. We need at least K, but ask for headroom
/// so that ranking ties and the K cap are not artificially conflated.
const CONTEXT_LIMIT: usize = 8;
const SNIPPETS_PER_FILE: usize = 1;

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
#[derive(Debug, Clone, Serialize)]
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
pub fn run(
    manifest_path: &Path,
    repos_dir: &Path,
    k_override: Option<usize>,
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
        issues.push(evaluate_issue(issue, repos_dir, k)?);
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

/// Evaluate a single issue. Indexes the repo at `<repos_dir>/<owner>/<name>`
/// and runs agent-context.
fn evaluate_issue(issue: &Issue, repos_dir: &Path, k: usize) -> Result<IssueResult> {
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

    let index = indexer::build_index(&repo_path)
        .with_context(|| format!("failed to index {}", repo_path.display()))?;
    let context = query::build_context(
        &repo_path,
        &index,
        &issue.task,
        CONTEXT_LIMIT,
        SNIPPETS_PER_FILE,
        false,
    )
    .with_context(|| format!("failed to build context for {}", issue.id))?;
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

/// Write the report JSON to disk. Creates the parent directory if missing.
pub fn write_report(out_path: &Path, report: &ModeAReport) -> Result<()> {
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
    let json = serde_json::to_string_pretty(report)?;
    fs::write(out_path, format!("{json}\n"))
        .with_context(|| format!("failed to write benchmark report to {}", out_path.display()))?;
    Ok(())
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
        Self {
            command: "bench-public",
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
}
