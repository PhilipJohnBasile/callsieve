use super::language::Language;

#[derive(Debug, Clone)]
pub struct RawSymbol {
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub visibility: String,
    pub parent: Option<String>,
    pub signature: String,
    pub doc: Option<String>,
}

pub fn extract_symbols(content: &str, language: Language) -> Vec<RawSymbol> {
    let lines: Vec<&str> = content.lines().collect();
    let mut symbols = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if let Some((name, kind, visibility)) = parse_line(line, language) {
            let start_line = index + 1;
            symbols.push(RawSymbol {
                name,
                kind,
                start_line,
                end_line: estimate_end_line(&lines, index, language),
                visibility,
                parent: None,
                signature: compact_signature(line),
                doc: previous_doc_comment(&lines, index, language),
            });
        }
    }

    assign_parents(&mut symbols);
    symbols
}

fn parse_line(line: &str, language: Language) -> Option<(String, String, String)> {
    match language {
        Language::Rust => parse_rust_line(line),
        Language::TypeScript | Language::JavaScript => parse_js_line(line),
        Language::Python => parse_python_line(line),
    }
}

fn parse_rust_line(line: &str) -> Option<(String, String, String)> {
    let trimmed = line.trim_start();
    let exported = trimmed.starts_with("pub ");
    let mut rest = strip_rust_visibility(trimmed);
    rest = rest.strip_prefix("async ").unwrap_or(rest).trim_start();

    for (prefix, kind) in [
        ("fn ", "function"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("trait ", "trait"),
        ("impl ", "impl"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                if exported { "public" } else { "private" }.to_string(),
            ));
        }
    }

    None
}

fn strip_rust_visibility(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix("pub ") {
        return rest.trim_start();
    }

    if line.starts_with("pub(")
        && let Some(end) = line.find(')')
    {
        return line[end + 1..].trim_start();
    }

    line
}

fn parse_js_line(line: &str) -> Option<(String, String, String)> {
    let mut rest = line.trim_start();
    let mut visibility = "local";

    if let Some(after_export) = rest.strip_prefix("export default ") {
        rest = after_export.trim_start();
        visibility = "exported";
    } else if let Some(after_export) = rest.strip_prefix("export ") {
        rest = after_export.trim_start();
        visibility = "exported";
    }

    rest = rest.strip_prefix("async ").unwrap_or(rest).trim_start();

    for (prefix, kind) in [
        ("function ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("type ", "type"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                visibility.to_string(),
            ));
        }
    }

    for prefix in ["const ", "let ", "var "] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            let kind = if after_prefix.contains("=>") || after_prefix.contains("function") {
                "function"
            } else {
                "constant"
            };
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                visibility.to_string(),
            ));
        }
    }

    if let Some(name) = parse_js_method(rest) {
        return Some((name, "method".to_string(), visibility.to_string()));
    }

    None
}

fn parse_python_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();

    for (prefix, kind) in [
        ("async def ", "function"),
        ("def ", "function"),
        ("class ", "class"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            let name = take_identifier(after_prefix)?;
            let visibility = if name.starts_with('_') {
                "private"
            } else {
                "public"
            };
            return Some((name, kind.to_string(), visibility.to_string()));
        }
    }

    None
}

fn take_identifier(input: &str) -> Option<String> {
    let identifier: String = input
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect();

    if identifier.is_empty() {
        None
    } else {
        Some(identifier)
    }
}

fn parse_js_method(line: &str) -> Option<String> {
    let rest = line.strip_prefix("async ").unwrap_or(line).trim_start();
    let name = take_identifier(rest)?;
    if matches!(
        name.as_str(),
        "if" | "for" | "while" | "switch" | "catch" | "function" | "return"
    ) {
        return None;
    }

    let after_name = rest[name.len()..].trim_start();
    (after_name.starts_with('(') && (after_name.contains('{') || after_name.contains("=>")))
        .then_some(name)
}

fn assign_parents(symbols: &mut [RawSymbol]) {
    for index in 0..symbols.len() {
        let parent = symbols[..index]
            .iter()
            .rev()
            .find(|candidate| {
                candidate.start_line < symbols[index].start_line
                    && candidate.end_line >= symbols[index].end_line
                    && matches!(
                        candidate.kind.as_str(),
                        "class" | "impl" | "struct" | "enum" | "trait" | "interface"
                    )
            })
            .map(|candidate| candidate.name.clone());

        symbols[index].parent = parent;
    }
}

fn estimate_end_line(lines: &[&str], start_index: usize, language: Language) -> usize {
    match language {
        Language::Python => estimate_python_end_line(lines, start_index),
        Language::Rust | Language::TypeScript | Language::JavaScript => {
            estimate_curly_end_line(lines, start_index)
        }
    }
}

fn estimate_python_end_line(lines: &[&str], start_index: usize) -> usize {
    let start_indent = indentation(lines[start_index]);
    let mut last_non_empty = start_index + 1;

    for (index, line) in lines.iter().enumerate().skip(start_index + 1) {
        if line.trim().is_empty() {
            continue;
        }

        if indentation(line) <= start_indent {
            break;
        }

        last_non_empty = index + 1;
    }

    last_non_empty
}

fn estimate_curly_end_line(lines: &[&str], start_index: usize) -> usize {
    let mut depth = 0_i32;
    let mut saw_open = false;

    for (index, line) in lines.iter().enumerate().skip(start_index) {
        for character in line.chars() {
            match character {
                '{' => {
                    depth += 1;
                    saw_open = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
        }

        if saw_open && depth <= 0 {
            return index + 1;
        }
    }

    start_index + 1
}

fn indentation(line: &str) -> usize {
    line.chars()
        .take_while(|character| character.is_whitespace())
        .map(|character| if character == '\t' { 4 } else { 1 })
        .sum()
}

fn compact_signature(line: &str) -> String {
    let signature = line.trim();
    if signature.len() > 160 {
        format!("{}...", &signature[..157])
    } else {
        signature.to_string()
    }
}

fn previous_doc_comment(lines: &[&str], index: usize, language: Language) -> Option<String> {
    if index == 0 {
        return None;
    }

    let previous = lines[index - 1].trim();
    let doc = match language {
        Language::Rust if previous.starts_with("///") => previous.trim_start_matches("///"),
        Language::Python if previous.starts_with('#') => previous.trim_start_matches('#'),
        Language::TypeScript | Language::JavaScript
            if previous.starts_with("//") || previous.starts_with('*') =>
        {
            previous.trim_start_matches('/').trim_start_matches('*')
        }
        _ => return None,
    };

    let doc = doc.trim();
    (!doc.is_empty()).then(|| doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_rust_symbols() {
        let symbols = extract_symbols(
            "pub struct User {}\nfn helper() {}\nimpl User {\n  pub fn name(&self) {}\n}\n",
            Language::Rust,
        );

        assert!(symbols.iter().any(|symbol| symbol.name == "User"));
        assert!(symbols.iter().any(|symbol| symbol.name == "helper"));
        assert!(symbols.iter().any(|symbol| symbol.name == "name"));
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "name" && symbol.parent.as_deref() == Some("User"))
        );
    }

    #[test]
    fn extracts_python_symbols() {
        let symbols = extract_symbols(
            "class User:\n    pass\n\ndef create_user():\n    return User()\n",
            Language::Python,
        );

        assert!(symbols.iter().any(|symbol| symbol.name == "User"));
        assert!(symbols.iter().any(|symbol| symbol.name == "create_user"));
    }

    #[test]
    fn extracts_python_method_parent() {
        let symbols = extract_symbols(
            "class User:\n    def login(self):\n        return True\n",
            Language::Python,
        );

        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "login" && symbol.parent.as_deref() == Some("User"))
        );
    }

    #[test]
    fn extracts_javascript_symbols() {
        let symbols = extract_symbols(
            "export function createSession() {}\nconst refreshSession = () => null;\nexport const token = 'x';\nclass Session {\n  renew() {}\n}\n",
            Language::TypeScript,
        );

        assert!(symbols.iter().any(|symbol| symbol.name == "createSession"));
        assert!(symbols.iter().any(|symbol| symbol.name == "refreshSession"));
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "token" && symbol.kind == "constant")
        );
        assert!(symbols.iter().any(|symbol| symbol.name == "Session"));
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "renew"
                    && symbol.parent.as_deref() == Some("Session"))
        );
    }
}
