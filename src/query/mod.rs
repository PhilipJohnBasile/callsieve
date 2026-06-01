pub mod formatter;
pub mod ranker;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::Result;
use serde::Serialize;

use crate::{
    indexer::language::Language,
    store::{CodeIndex, FileRecord, SymbolRecord},
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_tests: Vec<RelatedTest>,
    why: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ContextStats {
    candidate_matches: usize,
    selected_files: usize,
    selected_symbols: usize,
    related_tests: usize,
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
        self.graph_score += score;
        if self.seen_why.insert(why.clone()) {
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
