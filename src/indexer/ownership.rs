//! CODEOWNERS parsing and per-file ownership resolution.
//!
//! Implements canonical GitHub CODEOWNERS matching semantics:
//! - last matching pattern wins (later rules override earlier ones),
//! - glob patterns (`*`, `**`, `/path/`, `*.ext`) are supported,
//! - owners may be `@username`, `@org/team`, or email addresses,
//! - blank lines and `#` comment lines are ignored.
//!
//! Scope: parsing + resolution only. No git blame, no runtime context.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// File names searched (in order) when locating a CODEOWNERS file at the
/// repository root. The first existing file wins; absence is not an error.
pub const CODEOWNERS_CANDIDATES: &[&str] = &[
    ".github/CODEOWNERS",
    "CODEOWNERS",
    "docs/CODEOWNERS",
    ".gitlab/CODEOWNERS",
];

/// Per-file ownership computed from CODEOWNERS.
///
/// `owners` holds `@username` and email entries; `teams` holds `@org/team`
/// entries. Both are deduplicated and kept in the order they appear in the
/// matching rule.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ownership {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<String>,
}

impl Ownership {
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty() && self.teams.is_empty()
    }
}

/// One CODEOWNERS rule: a glob pattern plus the owners it assigns.
#[derive(Debug, Clone)]
struct Rule {
    pattern: String,
    owners: Vec<String>,
}

/// Parsed CODEOWNERS rule set, in source order. Matching scans in reverse so
/// the last matching rule wins (canonical GitHub semantics).
#[derive(Debug, Clone, Default)]
pub struct OwnershipResolver {
    rules: Vec<Rule>,
}

impl OwnershipResolver {
    /// Look for a CODEOWNERS file under `root` in the canonical search order
    /// and parse it. Returns `None` when no candidate exists, so callers can
    /// treat "no CODEOWNERS" identically to "empty resolver" without an error.
    pub fn from_repo_root(root: &Path) -> Option<Self> {
        for relative in CODEOWNERS_CANDIDATES {
            let candidate = root.join(relative);
            if candidate.is_file() {
                let content = std::fs::read_to_string(&candidate).ok()?;
                return Some(Self::parse(&content));
            }
        }
        None
    }

    /// Parse CODEOWNERS text. Blank lines and `#` comments are skipped; every
    /// non-comment line is treated as `<pattern> <owner...>`.
    pub fn parse(content: &str) -> Self {
        let mut rules = Vec::new();
        for raw_line in content.lines() {
            // Strip trailing comments after `#`, but only when the `#` is not
            // escaped. CODEOWNERS does not document `\#` escapes, so we keep it
            // simple: `#` always starts a comment.
            let line = match raw_line.split_once('#') {
                Some((before, _)) => before,
                None => raw_line,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut tokens = line.split_whitespace();
            let Some(pattern) = tokens.next() else {
                continue;
            };
            let owners: Vec<String> = tokens
                .filter(|token| !token.is_empty())
                .map(str::to_string)
                .collect();
            rules.push(Rule {
                pattern: pattern.to_string(),
                owners,
            });
        }
        Self { rules }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Resolve ownership for a single repo-relative path. The path uses `/`
    /// separators; callers normalize Windows paths before calling.
    pub fn ownership_for(&self, path: &str) -> Ownership {
        let normalized = path.trim_start_matches('/');
        for rule in self.rules.iter().rev() {
            if pattern_matches(&rule.pattern, normalized) {
                return classify_owners(&rule.owners);
            }
        }
        Ownership::default()
    }
}

fn classify_owners(raw: &[String]) -> Ownership {
    let mut owners = Vec::new();
    let mut teams = Vec::new();
    for entry in raw {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if is_team(entry) {
            if !teams.iter().any(|existing: &String| existing == entry) {
                teams.push(entry.to_string());
            }
        } else if !owners.iter().any(|existing: &String| existing == entry) {
            owners.push(entry.to_string());
        }
    }
    Ownership { owners, teams }
}

fn is_team(entry: &str) -> bool {
    // `@org/team` form. `@username` and email entries fall into `owners`.
    entry.starts_with('@') && entry.contains('/')
}

/// Match a CODEOWNERS glob pattern against a repo-relative path using GitHub
/// semantics:
///   - `*` matches one path segment (no `/`).
///   - `**` matches any number of segments (including zero).
///   - A leading `/` anchors the pattern at the repo root.
///   - A trailing `/` matches everything inside that directory.
///   - A bare name (no `/` other than a trailing one) matches anywhere in the
///     tree.
fn pattern_matches(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }

    // Special case: `*` (or `**`) by itself matches everything.
    if pattern == "*" || pattern == "**" {
        return true;
    }

    let anchored = pattern.starts_with('/');
    let dir_only = pattern.ends_with('/');
    let trimmed = pattern.trim_start_matches('/').trim_end_matches('/');
    let contains_slash = trimmed.contains('/');

    if !anchored && !contains_slash {
        // Bare name like `*.rs` or `Makefile` matches any path segment.
        if dir_only {
            // `name/` (no other slashes) - match any directory of that name.
            return path
                .split('/')
                .any(|segment| glob_segment_matches(trimmed, segment));
        }
        return path
            .rsplit('/')
            .next()
            .map(|file_name| glob_segment_matches(trimmed, file_name))
            .unwrap_or(false)
            || path
                .split('/')
                .any(|segment| glob_segment_matches(trimmed, segment));
    }

    if dir_only {
        // Directory match: the path must live inside this directory.
        let prefix_pattern = format!("{trimmed}/**");
        return glob_path_matches(&prefix_pattern, path);
    }

    // Otherwise: exact glob match anchored at repo root.
    glob_path_matches(trimmed, path)
}

/// Match a single path segment against a glob with `*` and `?` support.
fn glob_segment_matches(pattern: &str, segment: &str) -> bool {
    glob_match(pattern, segment, false)
}

/// Match a full repo-relative path against a glob with `*`, `**`, and `?`
/// support.
fn glob_path_matches(pattern: &str, path: &str) -> bool {
    glob_match(pattern, path, true)
}

/// Backtracking glob matcher.
///
/// `allow_double_star_and_slash`:
///   - `true`  -> `*` matches any chars except `/`; `**` matches any chars including `/`.
///   - `false` -> `*` matches any chars (segment context).
fn glob_match(pattern: &str, text: &str, allow_double_star_and_slash: bool) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    glob_match_recursive(
        &pattern_chars,
        0,
        &text_chars,
        0,
        allow_double_star_and_slash,
    )
}

fn glob_match_recursive(
    pattern: &[char],
    mut pi: usize,
    text: &[char],
    mut ti: usize,
    allow_double_star_and_slash: bool,
) -> bool {
    while pi < pattern.len() {
        match pattern[pi] {
            '*' => {
                let is_double =
                    allow_double_star_and_slash && pi + 1 < pattern.len() && pattern[pi + 1] == '*';
                if is_double {
                    // `**` consumes any chars including `/`. Skip trailing `/`
                    // immediately after `**/` so `src/**/foo` matches `src/foo`.
                    let mut next_pi = pi + 2;
                    if next_pi < pattern.len() && pattern[next_pi] == '/' {
                        // Try matching with the `/` consumed (zero intermediate
                        // segments) first, then fall through to the general
                        // multi-segment expansion.
                        if glob_match_recursive(
                            pattern,
                            next_pi + 1,
                            text,
                            ti,
                            allow_double_star_and_slash,
                        ) {
                            return true;
                        }
                        next_pi += 1;
                    }
                    // General case: `**` consumes 0..=remaining characters.
                    loop {
                        if glob_match_recursive(
                            pattern,
                            next_pi,
                            text,
                            ti,
                            allow_double_star_and_slash,
                        ) {
                            return true;
                        }
                        if ti >= text.len() {
                            return false;
                        }
                        ti += 1;
                    }
                } else {
                    // Single `*`: matches any number of chars except `/` (when
                    // `/` is significant).
                    let next_pi = pi + 1;
                    loop {
                        if glob_match_recursive(
                            pattern,
                            next_pi,
                            text,
                            ti,
                            allow_double_star_and_slash,
                        ) {
                            return true;
                        }
                        if ti >= text.len() {
                            return false;
                        }
                        if allow_double_star_and_slash && text[ti] == '/' {
                            return false;
                        }
                        ti += 1;
                    }
                }
            }
            '?' => {
                if ti >= text.len() {
                    return false;
                }
                if allow_double_star_and_slash && text[ti] == '/' {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            literal => {
                if ti >= text.len() || text[ti] != literal {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_simple_rules_and_ignores_comments_and_blanks() {
        let resolver = OwnershipResolver::parse(
            "# leading comment\n\
             \n\
             *       @global-owner\n\
             *.rs    @rust-team @octocat\n\
             # inline rule below\n\
             /docs/  docs@example.com\n",
        );
        assert!(!resolver.is_empty());
        assert_eq!(resolver.rules.len(), 3);
    }

    #[test]
    fn last_matching_rule_wins() {
        let resolver = OwnershipResolver::parse(
            "*           @everyone\n\
             *.rs        @rust-team\n\
             src/cli.rs  @cli-owner\n",
        );
        let ownership = resolver.ownership_for("src/cli.rs");
        assert_eq!(ownership.owners, vec!["@cli-owner".to_string()]);
        assert!(ownership.teams.is_empty());
    }

    #[test]
    fn earlier_rules_apply_when_later_rules_do_not_match() {
        let resolver = OwnershipResolver::parse(
            "*           @everyone\n\
             *.rs        @rust-team\n\
             src/cli.rs  @cli-owner\n",
        );
        let ownership = resolver.ownership_for("src/query/mod.rs");
        assert_eq!(ownership.owners, vec!["@rust-team".to_string()]);
    }

    #[test]
    fn falls_back_to_wildcard_when_no_specific_rule_matches() {
        let resolver = OwnershipResolver::parse(
            "*           @everyone\n\
             *.rs        @rust-team\n",
        );
        let ownership = resolver.ownership_for("README.md");
        assert_eq!(ownership.owners, vec!["@everyone".to_string()]);
    }

    #[test]
    fn classifies_team_owner_and_email_entries() {
        let resolver = OwnershipResolver::parse("* @org/team @user docs@example.com\n");
        let ownership = resolver.ownership_for("anything.rs");
        assert_eq!(ownership.teams, vec!["@org/team".to_string()]);
        assert_eq!(
            ownership.owners,
            vec!["@user".to_string(), "docs@example.com".to_string()]
        );
    }

    #[test]
    fn directory_pattern_matches_files_inside() {
        let resolver = OwnershipResolver::parse("/src/indexer/  @indexer-team\n");
        let ownership = resolver.ownership_for("src/indexer/mod.rs");
        assert_eq!(ownership.owners, vec!["@indexer-team".to_string()]);
        let ownership_outside = resolver.ownership_for("src/query/mod.rs");
        assert!(ownership_outside.is_empty());
    }

    #[test]
    fn anchored_pattern_matches_only_from_root() {
        let resolver = OwnershipResolver::parse("/cli.rs @root-cli\n");
        assert_eq!(
            resolver.ownership_for("cli.rs").owners,
            vec!["@root-cli".to_string()],
        );
        assert!(resolver.ownership_for("src/cli.rs").is_empty());
    }

    #[test]
    fn bare_name_matches_anywhere_in_tree() {
        let resolver = OwnershipResolver::parse("*.md @docs-team\n");
        let a = resolver.ownership_for("README.md");
        let b = resolver.ownership_for("docs/INSTALL.md");
        assert_eq!(a.owners, vec!["@docs-team".to_string()]);
        assert_eq!(b.owners, vec!["@docs-team".to_string()]);
    }

    #[test]
    fn double_star_matches_any_depth() {
        let resolver = OwnershipResolver::parse("/src/**/lsp.rs @lsp-team\n");
        assert_eq!(
            resolver.ownership_for("src/indexer/lsp.rs").owners,
            vec!["@lsp-team".to_string()],
        );
        assert_eq!(
            resolver.ownership_for("src/lsp.rs").owners,
            vec!["@lsp-team".to_string()],
        );
        assert!(resolver.ownership_for("src/indexer/mod.rs").is_empty());
    }

    #[test]
    fn single_star_does_not_cross_directory_boundary() {
        let resolver = OwnershipResolver::parse("/src/*.rs @top-rs\n");
        assert_eq!(
            resolver.ownership_for("src/cli.rs").owners,
            vec!["@top-rs".to_string()],
        );
        assert!(resolver.ownership_for("src/indexer/mod.rs").is_empty());
    }

    #[test]
    fn comments_after_pattern_are_stripped() {
        let resolver = OwnershipResolver::parse("*.rs @rust-team # everyone owns rust\n");
        let ownership = resolver.ownership_for("src/cli.rs");
        assert_eq!(ownership.owners, vec!["@rust-team".to_string()]);
    }

    #[test]
    fn rule_without_owners_clears_previous_match() {
        // Canonical CODEOWNERS allows rules with no owners; they still match
        // and produce an empty owner set, effectively clearing inherited
        // ownership for that path. We document this behavior with a test.
        let resolver = OwnershipResolver::parse(
            "*       @everyone\n\
             secret/ \n",
        );
        let ownership = resolver.ownership_for("secret/key.txt");
        assert!(ownership.is_empty());
        // Non-secret files keep the fallback owner.
        let other = resolver.ownership_for("README.md");
        assert_eq!(other.owners, vec!["@everyone".to_string()]);
    }

    #[test]
    fn reads_first_existing_codeowners_in_canonical_order() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".github")).unwrap();
        fs::write(temp.path().join(".github/CODEOWNERS"), "* @github-owner\n").unwrap();
        // A second candidate should be ignored because .github/CODEOWNERS wins.
        fs::write(temp.path().join("CODEOWNERS"), "* @root-owner\n").unwrap();

        let resolver = OwnershipResolver::from_repo_root(temp.path()).expect("found CODEOWNERS");
        assert_eq!(
            resolver.ownership_for("anything.rs").owners,
            vec!["@github-owner".to_string()],
        );
    }

    #[test]
    fn returns_none_when_no_codeowners_file_exists() {
        let temp = tempfile::tempdir().unwrap();
        assert!(OwnershipResolver::from_repo_root(temp.path()).is_none());
    }

    #[test]
    fn reads_gitlab_codeowners_when_other_paths_absent() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".gitlab")).unwrap();
        fs::write(
            temp.path().join(".gitlab/CODEOWNERS"),
            "*.py @python-team\n",
        )
        .unwrap();
        let resolver = OwnershipResolver::from_repo_root(temp.path()).expect("found CODEOWNERS");
        assert_eq!(
            resolver.ownership_for("svc/app.py").owners,
            vec!["@python-team".to_string()],
        );
    }

    #[test]
    fn reads_docs_codeowners_when_other_paths_absent() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/CODEOWNERS"), "*.md @docs-team\n").unwrap();
        let resolver = OwnershipResolver::from_repo_root(temp.path()).expect("found CODEOWNERS");
        assert_eq!(
            resolver.ownership_for("README.md").owners,
            vec!["@docs-team".to_string()],
        );
    }
}
