//! Code skeletonization: render a symbol as its signature with the body
//! collapsed to a `{ … }` (brace languages) or `…` (indentation languages)
//! marker.
//!
//! Borrowed from llmtrim's code compressor. An agent scanning a file's shape
//! rarely needs full function bodies; a skeleton shows every signature at a
//! fraction of the token cost. This works on the 1-based line ranges CallSieve
//! already derives from tree-sitter symbol extraction, so it adds no new
//! parsing dependency and stays deterministic.

/// A signature-only view of one symbol with its body elided.
#[derive(Debug, PartialEq, Eq)]
pub struct SymbolSkeleton {
    /// 1-based inclusive line span of the signature header kept in `text`.
    pub lines: [usize; 2],
    /// Signature header plus a body-elision marker.
    pub text: String,
    /// Number of body lines collapsed (0 when nothing was elided).
    pub omitted_lines: usize,
}

/// Builds a skeleton for the symbol spanning `start_line..=end_line` (1-based)
/// within `lines`. Returns `None` only when `lines` is empty.
///
/// When the symbol has no elidable body — one-liners, plain declarations, or a
/// body that does not span its own lines — the full span is returned with
/// `omitted_lines == 0`, so callers always get usable text.
pub fn skeletonize(lines: &[&str], start_line: usize, end_line: usize) -> Option<SymbolSkeleton> {
    if lines.is_empty() {
        return None;
    }
    let start = start_line.max(1).min(lines.len());
    let end = end_line.max(start).min(lines.len());
    let span = &lines[start - 1..end];

    if let Some((header_offset, brace_col)) = brace_header(span) {
        let header_line = start + header_offset;
        // Body lines live strictly after the brace line, up to `end`.
        if end > header_line {
            let mut text = String::new();
            for line in &span[..header_offset] {
                text.push_str(line);
                text.push('\n');
            }
            // Keep everything up to and including the opening brace, then elide.
            text.push_str(&span[header_offset][..=brace_col]);
            text.push_str(" … }");
            return Some(SymbolSkeleton {
                lines: [start, header_line],
                text,
                omitted_lines: end - header_line,
            });
        }
    } else if let Some(header_offset) = colon_header(span) {
        let header_line = start + header_offset;
        if end > header_line {
            let mut text: String = span[..=header_offset].join("\n");
            text.push_str("\n    …");
            return Some(SymbolSkeleton {
                lines: [start, header_line],
                text,
                omitted_lines: end - header_line,
            });
        }
    }

    // Nothing elidable: return the span verbatim.
    Some(SymbolSkeleton {
        lines: [start, end],
        text: span.join("\n"),
        omitted_lines: 0,
    })
}

/// First line in the span that opens a brace body, as `(line_offset, column)`
/// of the opening `{`. A body opener is a line whose trimmed content ends with
/// `{`, which avoids matching braces inside strings, f-strings, or inline
/// initializers mid-signature.
fn brace_header(span: &[&str]) -> Option<(usize, usize)> {
    span.iter().enumerate().find_map(|(offset, line)| {
        line.trim_end()
            .ends_with('{')
            .then(|| line.rfind('{').map(|col| (offset, col)))
            .flatten()
    })
}

/// First line in the span that ends a declaration with a colon (Python-style
/// `def`/`class` headers). Ignores trailing comments and whitespace.
fn colon_header(span: &[&str]) -> Option<usize> {
    span.iter().enumerate().find_map(|(offset, line)| {
        let code = match line.split_once('#') {
            Some((before, _)) => before,
            None => line,
        };
        code.trim_end().ends_with(':').then_some(offset)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        text.lines().collect()
    }

    #[test]
    fn elides_rust_function_body() {
        let src = "pub fn add(a: i32, b: i32) -> i32 {\n    let sum = a + b;\n    sum\n}";
        let skel = skeletonize(&lines(src), 1, 4).unwrap();
        assert_eq!(skel.text, "pub fn add(a: i32, b: i32) -> i32 { … }");
        assert_eq!(skel.lines, [1, 1]);
        assert_eq!(skel.omitted_lines, 3);
    }

    #[test]
    fn keeps_multiline_signature_before_eliding() {
        let src = "function build(\n  a,\n  b,\n) {\n  return a + b;\n}";
        let skel = skeletonize(&lines(src), 1, 6).unwrap();
        assert_eq!(skel.text, "function build(\n  a,\n  b,\n) { … }");
        assert_eq!(skel.lines, [1, 4]);
        assert_eq!(skel.omitted_lines, 2);
    }

    #[test]
    fn elides_python_body_with_colon_header() {
        let src = "def greet(name):\n    msg = f\"hi {name}\"\n    return msg";
        let skel = skeletonize(&lines(src), 1, 3).unwrap();
        assert_eq!(skel.text, "def greet(name):\n    …");
        assert_eq!(skel.omitted_lines, 2);
    }

    #[test]
    fn one_liner_is_returned_verbatim() {
        let src = "pub const MAX: usize = 120;";
        let skel = skeletonize(&lines(src), 1, 1).unwrap();
        assert_eq!(skel.text, "pub const MAX: usize = 120;");
        assert_eq!(skel.omitted_lines, 0);
    }

    #[test]
    fn empty_body_brace_on_last_line_is_verbatim() {
        let src = "fn noop() {}";
        let skel = skeletonize(&lines(src), 1, 1).unwrap();
        assert_eq!(skel.text, "fn noop() {}");
        assert_eq!(skel.omitted_lines, 0);
    }
}
