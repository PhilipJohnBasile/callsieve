use super::{language::Language, tree_sitter_symbols};

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
    if !language.is_code() {
        return Vec::new();
    }

    if let Some(symbols) = tree_sitter_symbols::extract_symbols(content, language)
        && !symbols.is_empty()
    {
        return symbols;
    }

    extract_symbols_fallback(content, language)
}

fn extract_symbols_fallback(content: &str, language: Language) -> Vec<RawSymbol> {
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
        Language::Php => parse_php_line(line),
        Language::Go => parse_go_line(line),
        Language::Java => parse_java_line(line),
        Language::CSharp => parse_csharp_line(line),
        Language::C => parse_c_line(line),
        Language::Cpp => parse_cpp_line(line),
        Language::Ruby => parse_ruby_line(line),
        Language::Kotlin => parse_kotlin_line(line),
        Language::Swift => parse_swift_line(line),
        Language::Scala => parse_scala_line(line),
        Language::Dart => parse_dart_line(line),
        Language::Lua => parse_lua_line(line),
        Language::Shell => parse_shell_line(line),
        Language::Markdown | Language::Json | Language::Toml | Language::Yaml | Language::Text => {
            None
        }
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
        ("mod ", "module"),
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
            let name = take_identifier(after_prefix)?;
            let kind = if is_react_component_candidate(&name, after_prefix) {
                "component"
            } else if after_prefix.contains("=>") || after_prefix.contains("function") {
                "function"
            } else {
                "constant"
            };
            return Some((name, kind.to_string(), visibility.to_string()));
        }
    }

    if let Some(name) = parse_js_method(rest) {
        return Some((name, "method".to_string(), visibility.to_string()));
    }

    None
}

fn is_react_component_candidate(name: &str, rest: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|character| character.is_ascii_uppercase())
        && (rest.contains("=> <") || rest.contains("React.createElement"))
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

fn parse_php_line(line: &str) -> Option<(String, String, String)> {
    let trimmed = line.trim_start().trim_start_matches("<?php").trim_start();
    let visibility = visibility_from_modifiers(trimmed, "public");
    let rest = strip_leading_modifiers(
        trimmed,
        &[
            "abstract",
            "final",
            "readonly",
            "public",
            "private",
            "protected",
            "static",
        ],
    );

    for (prefix, kind) in [
        ("function ", "function"),
        ("class ", "class"),
        ("interface ", "interface"),
        ("trait ", "trait"),
        ("enum ", "enum"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                visibility.to_string(),
            ));
        }
    }

    None
}

fn parse_go_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();

    if let Some(after_prefix) = rest.strip_prefix("func ") {
        let function_name = if let Some(receiver_stripped) = after_prefix.strip_prefix('(') {
            let receiver_end = receiver_stripped.find(')')?;
            receiver_stripped[receiver_end + 1..].trim_start()
        } else {
            after_prefix
        };
        let name = take_identifier(function_name)?;
        return Some((
            name.clone(),
            "function".to_string(),
            go_visibility(&name).to_string(),
        ));
    }

    if let Some(after_prefix) = rest.strip_prefix("type ") {
        let name = take_identifier(after_prefix)?;
        let kind = if after_prefix.contains(" struct") {
            "struct"
        } else if after_prefix.contains(" interface") {
            "interface"
        } else {
            "type"
        };
        return Some((
            name.clone(),
            kind.to_string(),
            go_visibility(&name).to_string(),
        ));
    }

    None
}

fn parse_java_line(line: &str) -> Option<(String, String, String)> {
    parse_c_style_line(
        line,
        &[
            "public",
            "private",
            "protected",
            "static",
            "final",
            "abstract",
            "synchronized",
            "native",
            "strictfp",
            "sealed",
            "non-sealed",
        ],
        &[
            ("class ", "class"),
            ("interface ", "interface"),
            ("enum ", "enum"),
            ("record ", "record"),
        ],
    )
}

fn parse_csharp_line(line: &str) -> Option<(String, String, String)> {
    parse_c_style_line(
        line,
        &[
            "public",
            "private",
            "protected",
            "internal",
            "static",
            "sealed",
            "abstract",
            "partial",
            "async",
            "readonly",
            "virtual",
            "override",
        ],
        &[
            ("class ", "class"),
            ("interface ", "interface"),
            ("enum ", "enum"),
            ("struct ", "struct"),
            ("record ", "record"),
        ],
    )
}

fn parse_c_line(line: &str) -> Option<(String, String, String)> {
    parse_c_style_line(
        line,
        &["static", "inline", "extern", "const", "volatile"],
        &[
            ("struct ", "struct"),
            ("enum ", "enum"),
            ("typedef ", "type"),
        ],
    )
}

fn parse_cpp_line(line: &str) -> Option<(String, String, String)> {
    parse_c_style_line(
        line,
        &[
            "public",
            "private",
            "protected",
            "static",
            "inline",
            "virtual",
            "constexpr",
            "consteval",
            "extern",
            "template",
        ],
        &[
            ("class ", "class"),
            ("struct ", "struct"),
            ("enum ", "enum"),
            ("namespace ", "module"),
            ("typedef ", "type"),
        ],
    )
}

fn parse_ruby_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("class ", "class"),
        ("module ", "module"),
        ("def ", "function"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            let mut name = take_qualified_identifier(after_prefix)?;
            if let Some(stripped) = name.strip_prefix("self.") {
                name = stripped.to_string();
            }
            return Some((
                last_qualified_part(&name),
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_kotlin_line(line: &str) -> Option<(String, String, String)> {
    let rest = strip_leading_modifiers(
        line.trim_start(),
        &[
            "public",
            "private",
            "protected",
            "internal",
            "open",
            "data",
            "sealed",
            "abstract",
            "final",
            "override",
            "suspend",
        ],
    );

    if let Some(after_prefix) = rest.strip_prefix("fun ") {
        return Some((
            take_identifier(after_prefix)?,
            "function".to_string(),
            visibility_from_modifiers(line.trim_start(), "public").to_string(),
        ));
    }

    for (prefix, kind) in [
        ("class ", "class"),
        ("object ", "object"),
        ("interface ", "interface"),
        ("enum class ", "enum"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                visibility_from_modifiers(line.trim_start(), "public").to_string(),
            ));
        }
    }

    None
}

fn parse_swift_line(line: &str) -> Option<(String, String, String)> {
    let rest = strip_leading_modifiers(
        line.trim_start(),
        &[
            "public",
            "private",
            "fileprivate",
            "internal",
            "open",
            "static",
            "final",
        ],
    );
    for (prefix, kind) in [
        ("func ", "function"),
        ("class ", "class"),
        ("struct ", "struct"),
        ("enum ", "enum"),
        ("protocol ", "interface"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                visibility_from_modifiers(line.trim_start(), "internal").to_string(),
            ));
        }
    }
    None
}

fn parse_scala_line(line: &str) -> Option<(String, String, String)> {
    let rest = strip_leading_modifiers(
        line.trim_start(),
        &[
            "private",
            "protected",
            "final",
            "sealed",
            "abstract",
            "case",
            "implicit",
        ],
    );
    for (prefix, kind) in [
        ("def ", "function"),
        ("class ", "class"),
        ("object ", "object"),
        ("trait ", "trait"),
        ("enum ", "enum"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                visibility_from_modifiers(line.trim_start(), "public").to_string(),
            ));
        }
    }
    None
}

fn parse_dart_line(line: &str) -> Option<(String, String, String)> {
    let rest = strip_leading_modifiers(line.trim_start(), &["abstract", "base", "final", "sealed"]);
    for (prefix, kind) in [
        ("class ", "class"),
        ("mixin ", "mixin"),
        ("enum ", "enum"),
        ("typedef ", "type"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    parse_c_style_method(rest, "public")
}

fn parse_lua_line(line: &str) -> Option<(String, String, String)> {
    let rest = line
        .trim_start()
        .strip_prefix("local ")
        .unwrap_or(line.trim_start())
        .trim_start();
    if let Some(after_prefix) = rest.strip_prefix("function ") {
        let name = take_qualified_identifier(after_prefix)?;
        return Some((
            last_qualified_part(&name),
            "function".to_string(),
            "local".to_string(),
        ));
    }
    None
}

fn parse_shell_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if let Some(after_prefix) = rest.strip_prefix("function ") {
        return Some((
            take_identifier(after_prefix)?,
            "function".to_string(),
            "local".to_string(),
        ));
    }
    if let Some(before_paren) = rest.split_once("()").map(|(name, _)| name.trim())
        && is_identifier(before_paren)
    {
        return Some((
            before_paren.to_string(),
            "function".to_string(),
            "local".to_string(),
        ));
    }
    None
}

fn parse_c_style_line(
    line: &str,
    modifiers: &[&str],
    type_prefixes: &[(&str, &str)],
) -> Option<(String, String, String)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('@') || trimmed.starts_with('#') {
        return None;
    }
    let visibility = visibility_from_modifiers(trimmed, "local");
    let rest = strip_leading_modifiers(trimmed, modifiers);

    for (prefix, kind) in type_prefixes {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                (*kind).to_string(),
                visibility.to_string(),
            ));
        }
    }

    parse_c_style_method(rest, visibility)
}

fn parse_c_style_method(line: &str, visibility: &str) -> Option<(String, String, String)> {
    if !line.contains('(') || line.ends_with(';') {
        return None;
    }
    let name = method_name_before_paren(line)?;
    Some((name, "function".to_string(), visibility.to_string()))
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

fn take_qualified_identifier(input: &str) -> Option<String> {
    let identifier: String = input
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric() || matches!(*character, '_' | '.' | ':' | '\\')
        })
        .collect();

    if identifier.is_empty() {
        None
    } else {
        Some(identifier)
    }
}

fn last_qualified_part(name: &str) -> String {
    name.rsplit(['.', ':', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or(name)
        .to_string()
}

fn is_identifier(input: &str) -> bool {
    let mut chars = input.chars();
    chars.next().is_some_and(|character| {
        character.is_ascii_alphabetic() || character == '_' || character == '$'
    }) && chars
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '$')
}

fn strip_leading_modifiers<'a>(mut input: &'a str, modifiers: &[&str]) -> &'a str {
    loop {
        let mut stripped = false;
        for modifier in modifiers {
            if let Some(rest) = input.strip_prefix(modifier)
                && rest
                    .chars()
                    .next()
                    .is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
            {
                input = rest.trim_start();
                stripped = true;
                break;
            }
        }
        if !stripped {
            return input;
        }
    }
}

fn visibility_from_modifiers(line: &str, default_visibility: &'static str) -> &'static str {
    for visibility in [
        "public",
        "private",
        "protected",
        "internal",
        "fileprivate",
        "open",
    ] {
        if line.split_whitespace().any(|part| part == visibility) {
            return visibility;
        }
    }
    default_visibility
}

fn go_visibility(name: &str) -> &'static str {
    if name
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_uppercase())
    {
        "exported"
    } else {
        "private"
    }
}

fn method_name_before_paren(line: &str) -> Option<String> {
    let paren_index = line.find('(')?;
    let before = line[..paren_index].trim_end();
    if before.is_empty()
        || before.contains('=')
        || before.ends_with(" if")
        || before.ends_with(" for")
        || before.ends_with(" while")
    {
        return None;
    }
    let name = before
        .rsplit(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '_' || character == '$')
        })
        .find(|part| !part.is_empty())?;
    if matches!(
        name,
        "if" | "for" | "while" | "switch" | "catch" | "return" | "sizeof" | "new"
    ) {
        return None;
    }
    Some(name.to_string())
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
        Language::Rust
        | Language::TypeScript
        | Language::JavaScript
        | Language::Php
        | Language::Go
        | Language::Java
        | Language::CSharp
        | Language::C
        | Language::Cpp
        | Language::Kotlin
        | Language::Swift
        | Language::Scala
        | Language::Dart => estimate_curly_end_line(lines, start_index),
        Language::Ruby | Language::Lua | Language::Shell => start_index + 1,
        Language::Markdown | Language::Json | Language::Toml | Language::Yaml | Language::Text => {
            start_index + 1
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
        Language::Ruby | Language::Shell if previous.starts_with('#') => {
            previous.trim_start_matches('#')
        }
        Language::TypeScript
        | Language::JavaScript
        | Language::Php
        | Language::Go
        | Language::Java
        | Language::CSharp
        | Language::C
        | Language::Cpp
        | Language::Kotlin
        | Language::Swift
        | Language::Scala
        | Language::Dart
        | Language::Lua
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
