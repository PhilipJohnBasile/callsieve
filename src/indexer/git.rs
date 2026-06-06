//! Per-file git behavioral signals: recency, commit frequency, author spread,
//! and churn over the last 90 days.
//!
//! This is the kind of "who knows this code / what changes often" context that
//! built-in agent context layers can't cheaply produce, because they don't keep
//! a persistent local index. We get it from a single `git log` pass parsed
//! in-process (no per-file fork), and degrade silently to no signal when the
//! tree isn't a git repo or `git` isn't installed.
//!
//! The signal is surfaced in the index and the read-first packet. Folding it
//! into the ranker as a recency/hotspot boost is a deliberate follow-up so the
//! retrieval benchmark can gate that change.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const DAY_SECONDS: u64 = 86_400;

/// Git activity for one file over the trailing 90 days.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitSignal {
    /// Unix timestamp of the most recent commit touching the file (within 90d).
    pub last_modified_unix: u64,
    /// Commits touching the file in the last 30 days.
    pub commits_30d: u32,
    /// Commits touching the file in the last 90 days.
    pub commits_90d: u32,
    /// Distinct commit authors touching the file in the last 90 days.
    pub distinct_authors_90d: u32,
    /// Lines added + deleted across those commits (a churn proxy).
    pub churn_90d: u64,
}

/// Collect per-file git signals keyed by repo-relative path. Empty when the
/// directory isn't a git repo, `git` is unavailable, or there is no recent
/// history.
pub fn collect_git_signals(root: &Path) -> BTreeMap<String, GitSignal> {
    collect_with_now(root, now_unix())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn collect_with_now(root: &Path, now: u64) -> BTreeMap<String, GitSignal> {
    // \x01 marks a commit header so we can tell it apart from numstat rows
    // regardless of author names. --no-renames keeps numstat paths single-valued.
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "log",
            "--since=90.days",
            "--no-renames",
            "--numstat",
            "--pretty=format:%x01%H%x09%an%x09%at",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => {
            parse_git_log(&String::from_utf8_lossy(&output.stdout), now)
        }
        _ => BTreeMap::new(),
    }
}

#[derive(Default)]
struct Accumulator {
    last_modified_unix: u64,
    commits_30d: u32,
    commits_90d: u32,
    authors: BTreeSet<String>,
    churn_90d: u64,
}

/// Pure parser over `git log --numstat` output with the `\x01`-prefixed header
/// format above. Pulled out so it can be unit-tested with synthetic timestamps.
fn parse_git_log(text: &str, now: u64) -> BTreeMap<String, GitSignal> {
    let cutoff_30 = now.saturating_sub(30 * DAY_SECONDS);
    let cutoff_90 = now.saturating_sub(90 * DAY_SECONDS);

    let mut per_file: BTreeMap<String, Accumulator> = BTreeMap::new();
    let mut author = String::new();
    let mut timestamp: u64 = 0;

    for line in text.lines() {
        if let Some(header) = line.strip_prefix('\u{1}') {
            let mut parts = header.splitn(3, '\t');
            let _hash = parts.next().unwrap_or("");
            author = parts.next().unwrap_or("").to_string();
            timestamp = parts
                .next()
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let added = fields.next().unwrap_or("");
        let deleted = fields.next().unwrap_or("");
        let Some(path) = fields.next() else {
            continue;
        };
        if path.is_empty() || timestamp < cutoff_90 {
            continue;
        }

        let entry = per_file.entry(path.to_string()).or_default();
        entry.commits_90d += 1;
        entry.churn_90d += added.parse::<u64>().unwrap_or(0) + deleted.parse::<u64>().unwrap_or(0);
        entry.last_modified_unix = entry.last_modified_unix.max(timestamp);
        if !author.is_empty() {
            entry.authors.insert(author.clone());
        }
        if timestamp >= cutoff_30 {
            entry.commits_30d += 1;
        }
    }

    per_file
        .into_iter()
        .map(|(path, acc)| {
            (
                path,
                GitSignal {
                    last_modified_unix: acc.last_modified_unix,
                    commits_30d: acc.commits_30d,
                    commits_90d: acc.commits_90d,
                    distinct_authors_90d: acc.authors.len() as u32,
                    churn_90d: acc.churn_90d,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_commits_authors_and_churn_within_windows() {
        let now = 100 * DAY_SECONDS;
        // Two commits touch src/a.rs (one recent, one 60 days old by different
        // authors); one old-but-within-90d commit touches src/b.rs.
        let recent = now - 5 * DAY_SECONDS;
        let mid = now - 60 * DAY_SECONDS;
        let log = format!(
            "\u{1}h1\talice\t{recent}\n\
             10\t2\tsrc/a.rs\n\
             \u{1}h2\tbob\t{mid}\n\
             3\t1\tsrc/a.rs\n\
             0\t0\tsrc/b.rs\n"
        );

        let signals = parse_git_log(&log, now);

        let a = signals.get("src/a.rs").expect("a.rs has signal");
        assert_eq!(a.commits_90d, 2);
        assert_eq!(a.commits_30d, 1, "only the 5-day-old commit is within 30d");
        assert_eq!(a.distinct_authors_90d, 2);
        assert_eq!(a.churn_90d, 16);
        assert_eq!(a.last_modified_unix, recent);

        let b = signals.get("src/b.rs").expect("b.rs has signal");
        assert_eq!(b.commits_90d, 1);
        assert_eq!(b.commits_30d, 0);
        assert_eq!(b.churn_90d, 0, "binary/no-change still counts as a commit");
    }

    #[test]
    fn drops_commits_older_than_ninety_days() {
        let now = 200 * DAY_SECONDS;
        let ancient = now - 120 * DAY_SECONDS;
        let log = format!("\u{1}h1\talice\t{ancient}\n5\t5\tsrc/old.rs\n");
        assert!(parse_git_log(&log, now).is_empty());
    }

    #[test]
    fn non_repo_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(collect_git_signals(tmp.path()).is_empty());
    }
}
