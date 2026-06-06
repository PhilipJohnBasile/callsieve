//! Stack-trace / error-log parsing for `agent-context --error <file>`.
//!
//! Built-in agents struggle to point a retrieval at the exact files a crash
//! implicates, because they don't keep a persistent local file index. CallSieve
//! does, so we can parse a pasted stack trace into `(path, line)` frames and
//! match them against the index to surface those files first.
//!
//! Dependency-free on purpose (no `regex`). We recognize the two dominant
//! shapes and ignore everything else:
//!   - Python:  `  File "path/to/x.py", line 42, in handler`
//!   - Generic: `path/to/x.rs:12:5`, `at foo (src/x.ts:10:3)`, `(Bar.java:42)`
//!
//! Matching is by path suffix (trace paths are often absolute) with a basename
//! fallback for bare filenames like Java's `Bar.java`.

use crate::store::CodeIndex;

/// One parsed stack-trace frame: a file path and, when present, a line number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    pub path: String,
    pub line: Option<usize>,
}

/// A frame resolved to an indexed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameMatch {
    pub file_id: String,
    pub line: Option<usize>,
}

/// Parse an error log into frames, de-duplicated on `(path, line)` and kept in
/// first-seen order (top of the stack first, which is usually most relevant).
pub fn parse_stack_trace(text: &str) -> Vec<StackFrame> {
    let mut frames = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for line in text.lines() {
        for frame in parse_line(line) {
            if seen.insert((frame.path.clone(), frame.line)) {
                frames.push(frame);
            }
        }
    }
    frames
}

/// Resolve frames to indexed files by path suffix (or basename for bare names),
/// de-duplicated on `file_id` (first match wins, preferring a known line).
pub fn match_frames(frames: &[StackFrame], index: &CodeIndex) -> Vec<FrameMatch> {
    let mut matches: Vec<FrameMatch> = Vec::new();
    for frame in frames {
        let frame_norm = normalize(&frame.path);
        let has_dir = frame_norm.contains('/');
        let frame_base = basename(&frame_norm);

        let mut best: Option<(usize, &str)> = None;
        for file in &index.files {
            let file_norm = normalize(&file.path);
            let hit = if has_dir {
                frame_norm.ends_with(&file_norm) || file_norm.ends_with(&frame_norm)
            } else {
                basename(&file_norm) == frame_base
            };
            if !hit {
                continue;
            }
            // Prefer the most specific (longest) shared suffix on a tie.
            let score = common_suffix_len(&frame_norm, &file_norm);
            if best.is_none_or(|(best_score, _)| score > best_score) {
                best = Some((score, file.id.as_str()));
            }
        }

        let Some((_, file_id)) = best else {
            continue;
        };
        if let Some(existing) = matches.iter_mut().find(|m| m.file_id == file_id) {
            if existing.line.is_none() {
                existing.line = frame.line;
            }
        } else {
            matches.push(FrameMatch {
                file_id: file_id.to_string(),
                line: frame.line,
            });
        }
    }
    matches
}

fn parse_line(line: &str) -> Vec<StackFrame> {
    if let Some(frame) = parse_python_frame(line) {
        return vec![frame];
    }
    line.split(|c: char| {
        c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '\'' | '"' | ',' | '<' | '>')
    })
    .filter_map(parse_path_token)
    .collect()
}

fn parse_python_frame(line: &str) -> Option<StackFrame> {
    let start = line.find("File \"")?;
    let rest = &line[start + "File \"".len()..];
    let end = rest.find('"')?;
    let path = rest[..end].to_string();
    if !looks_like_file_path(&path) {
        return None;
    }
    let after = &rest[end + 1..];
    let line_no = after.find("line ").and_then(|i| {
        let digits: String = after[i + "line ".len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse::<usize>().ok()
    });
    Some(StackFrame {
        path,
        line: line_no,
    })
}

fn parse_path_token(token: &str) -> Option<StackFrame> {
    if token.is_empty() || token.contains("://") {
        return None;
    }
    // Split off the first ':' as the path/line boundary. (Windows drive letters
    // like `C:` fail `looks_like_file_path` and are dropped; acceptable for now.)
    let (path, tail) = match token.find(':') {
        Some(i) => (&token[..i], &token[i + 1..]),
        None => (token, ""),
    };
    if !looks_like_file_path(path) {
        return None;
    }
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    Some(StackFrame {
        path: path.to_string(),
        line: digits.parse::<usize>().ok(),
    })
}

/// A path "looks like a file" if its last segment has an extension whose chars
/// are alphanumeric and include at least one letter (so version numbers like
/// `0.30` or `1.0` are not mistaken for files).
fn looks_like_file_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => {
            let ext = &name[i + 1..];
            ext.len() <= 8
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        }
        _ => false,
    }
}

fn normalize(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    replaced.strip_prefix("./").unwrap_or(&replaced).to_string()
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Length (in bytes) of the longest shared trailing path; used to prefer the
/// most specific candidate when several files match a frame.
fn common_suffix_len(a: &str, b: &str) -> usize {
    a.bytes()
        .rev()
        .zip(b.bytes().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{FileRecord, IndexMetadata};

    fn file(id: &str, path: &str) -> FileRecord {
        FileRecord {
            id: id.to_string(),
            path: path.to_string(),
            language: crate::indexer::language::Language::Rust,
            size_bytes: 0,
            line_count: 0,
            mtime: 0,
            content_hash: format!("fnv1a64:{id}"),
            is_test: false,
            is_config: false,
            module_path: String::new(),
            content_terms: Vec::new(),
            ownership: None,
            git: None,
        }
    }

    fn index(files: Vec<FileRecord>) -> CodeIndex {
        CodeIndex {
            schema_version: crate::indexer::SCHEMA_VERSION,
            root: ".".to_string(),
            metadata: IndexMetadata::default(),
            files,
            symbols: Vec::new(),
            imports: Vec::new(),
            references: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn parses_python_rust_js_and_java_shapes() {
        let trace = r#"
Traceback (most recent call last):
  File "/srv/app/src/auth/session.py", line 42, in resolve
    raise ValueError("bad")
    at handleRedirect (src/net/redirect.ts:10:3)
thread 'main' panicked at src/store/mod.rs:128:9
    at com.example.Bar.run(Bar.java:88)
version 1.0 should not match
"#;
        let frames = parse_stack_trace(trace);
        assert!(frames.contains(&StackFrame {
            path: "/srv/app/src/auth/session.py".to_string(),
            line: Some(42),
        }));
        assert!(frames.contains(&StackFrame {
            path: "src/net/redirect.ts".to_string(),
            line: Some(10),
        }));
        assert!(frames.contains(&StackFrame {
            path: "src/store/mod.rs".to_string(),
            line: Some(128),
        }));
        assert!(frames.contains(&StackFrame {
            path: "Bar.java".to_string(),
            line: Some(88),
        }));
        // "1.0" is a version, not a file.
        assert!(!frames.iter().any(|f| f.path == "1.0"));
    }

    #[test]
    fn matches_absolute_trace_path_to_relative_index_path() {
        let idx = index(vec![
            file("a", "src/auth/session.py"),
            file("b", "src/net/redirect.ts"),
            file("c", "src/unrelated.rs"),
        ]);
        let frames = vec![
            StackFrame {
                path: "/srv/app/src/auth/session.py".to_string(),
                line: Some(42),
            },
            StackFrame {
                path: "redirect.ts".to_string(),
                line: None,
            },
        ];
        let matches = match_frames(&frames, &idx);
        assert_eq!(
            matches,
            vec![
                FrameMatch {
                    file_id: "a".to_string(),
                    line: Some(42),
                },
                FrameMatch {
                    file_id: "b".to_string(),
                    line: None,
                },
            ]
        );
    }

    #[test]
    fn unmatched_frames_are_dropped() {
        let idx = index(vec![file("a", "src/auth/session.py")]);
        let frames = vec![StackFrame {
            path: "/other/project/main.go".to_string(),
            line: Some(1),
        }];
        assert!(match_frames(&frames, &idx).is_empty());
    }
}
