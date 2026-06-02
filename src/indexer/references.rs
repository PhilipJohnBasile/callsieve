use std::collections::BTreeSet;

use tree_sitter::{Node, Parser};

use super::language::Language;

#[derive(Debug, Clone)]
pub struct RawReference {
    pub target_name: String,
    pub kind: String,
    pub line: usize,
    pub edge_source: String,
    pub confidence: f64,
}

pub fn extract_references(
    content: &str,
    language: Language,
    candidate_names: &BTreeSet<String>,
) -> Vec<RawReference> {
    if candidate_names.is_empty() {
        return Vec::new();
    }

    if language.is_code()
        && let Some(references) = extract_tree_sitter_references(content, language, candidate_names)
    {
        return references;
    }

    let scan_content;
    let content = if language.is_code() {
        scan_content = strip_comments_and_strings(content, language);
        scan_content.as_str()
    } else {
        content
    };
    extract_references_fallback(content, candidate_names)
}

fn extract_references_fallback(
    content: &str,
    candidate_names: &BTreeSet<String>,
) -> Vec<RawReference> {
    let mut references = Vec::new();
    let mut seen = BTreeSet::new();

    for (line_index, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
        {
            continue;
        }

        for name in candidate_names {
            if name.len() < 3 || is_definition_line(line, name) {
                continue;
            }

            if let Some(kind) = reference_kind(line, name) {
                let line_number = line_index + 1;
                if seen.insert((line_number, name.clone(), kind.clone())) {
                    references.push(RawReference {
                        target_name: name.clone(),
                        kind,
                        line: line_number,
                        edge_source: "heuristic".to_string(),
                        confidence: 0.45,
                    });
                }
            }
        }
    }

    references
}

fn extract_tree_sitter_references(
    content: &str,
    language: Language,
    candidate_names: &BTreeSet<String>,
) -> Option<Vec<RawReference>> {
    let parser_language = parser_language(language)?;
    let mut parser = Parser::new();
    parser.set_language(&parser_language).ok()?;
    let tree = parser.parse(content, None)?;
    if tree.root_node().has_error() {
        return None;
    }

    let mut references = Vec::new();
    let mut seen = BTreeSet::new();
    collect_reference_nodes(
        tree.root_node(),
        content,
        candidate_names,
        &mut references,
        &mut seen,
    );
    Some(references)
}

fn parser_language(language: Language) -> Option<tree_sitter::Language> {
    match language {
        Language::JavaScript => Some(tree_sitter_javascript::LANGUAGE.into()),
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        Language::Markdown | Language::Json | Language::Toml | Language::Yaml | Language::Text => {
            None
        }
    }
}

fn collect_reference_nodes(
    node: Node<'_>,
    content: &str,
    candidate_names: &BTreeSet<String>,
    references: &mut Vec<RawReference>,
    seen: &mut BTreeSet<(usize, String, String)>,
) {
    if is_reference_identifier(node)
        && !is_definition_identifier(node)
        && !is_import_identifier(node)
        && let Ok(name) = node.utf8_text(content.as_bytes())
        && name.len() >= 3
        && candidate_names.contains(name)
    {
        let line_number = node.start_position().row + 1;
        let kind = reference_kind_after_node(content, node);
        if seen.insert((line_number, name.to_string(), kind.clone())) {
            references.push(RawReference {
                target_name: name.to_string(),
                kind,
                line: line_number,
                edge_source: "tree_sitter".to_string(),
                confidence: 0.8,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_reference_nodes(child, content, candidate_names, references, seen);
    }
}

fn is_reference_identifier(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier"
            | "property_identifier"
            | "field_identifier"
            | "shorthand_property_identifier"
            | "type_identifier"
    )
}

fn is_definition_identifier(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if !matches!(
        parent.kind(),
        "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "method_definition"
            | "interface_declaration"
            | "type_alias_declaration"
            | "variable_declarator"
            | "function_definition"
            | "class_definition"
            | "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "impl_item"
            | "const_item"
            | "mod_item"
    ) {
        return false;
    }

    ["name", "pattern", "type"]
        .into_iter()
        .filter_map(|field| parent.child_by_field_name(field))
        .any(|child| child.start_byte() == node.start_byte() && child.end_byte() == node.end_byte())
}

fn is_import_identifier(node: Node<'_>) -> bool {
    let mut current = node.parent();
    while let Some(node) = current {
        if matches!(
            node.kind(),
            "import_statement"
                | "import_specifier"
                | "import_clause"
                | "import_from_statement"
                | "aliased_import"
                | "use_declaration"
                | "scoped_use_list"
                | "use_as_clause"
        ) {
            return true;
        }
        current = node.parent();
    }
    false
}

fn reference_kind_after_node(content: &str, node: Node<'_>) -> String {
    let after = &content[node.end_byte()..];
    let is_call = after
        .chars()
        .find(|character| !character.is_whitespace())
        .is_some_and(|character| character == '(');

    if is_call { "call" } else { "reference" }.to_string()
}

fn reference_kind(line: &str, name: &str) -> Option<String> {
    let start = find_identifier(line, name)?;
    let after = &line[start + name.len()..];
    let is_call = after
        .chars()
        .find(|character| !character.is_whitespace())
        .is_some_and(|character| character == '(');

    Some(if is_call { "call" } else { "reference" }.to_string())
}

fn find_identifier(line: &str, name: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(relative_start) = line[offset..].find(name) {
        let start = offset + relative_start;
        let end = start + name.len();
        if is_boundary(line[..start].chars().next_back()) && is_boundary(line[end..].chars().next())
        {
            return Some(start);
        }
        offset = end;
    }

    None
}

fn is_boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| {
        !(character.is_ascii_alphanumeric() || character == '_' || character == '$')
    })
}

fn is_definition_line(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    let prefixes = [
        format!("function {name}"),
        format!("async function {name}"),
        format!("export function {name}"),
        format!("export async function {name}"),
        format!("class {name}"),
        format!("export class {name}"),
        format!("interface {name}"),
        format!("export interface {name}"),
        format!("type {name}"),
        format!("export type {name}"),
        format!("const {name}"),
        format!("export const {name}"),
        format!("let {name}"),
        format!("export let {name}"),
        format!("var {name}"),
        format!("export var {name}"),
        format!("def {name}"),
        format!("async def {name}"),
        format!("class {name}"),
        format!("fn {name}"),
        format!("pub fn {name}"),
        format!("struct {name}"),
        format!("pub struct {name}"),
        format!("enum {name}"),
        format!("pub enum {name}"),
        format!("trait {name}"),
        format!("pub trait {name}"),
    ];

    prefixes.iter().any(|prefix| trimmed.starts_with(prefix))
}

fn strip_comments_and_strings(content: &str, language: Language) -> String {
    let chars: Vec<char> = content.chars().collect();
    let mut output = String::with_capacity(content.len());
    let mut index = 0;
    let mut state = StripState::Normal;

    while index < chars.len() {
        let character = chars[index];
        match state {
            StripState::Normal => {
                if let Some(length) = line_comment_len(&chars, index, language) {
                    for _ in 0..length {
                        output.push(' ');
                    }
                    index += length;
                    state = StripState::LineComment;
                } else if let Some(length) = block_comment_len(&chars, index, language) {
                    for _ in 0..length {
                        output.push(' ');
                    }
                    index += length;
                    state = StripState::BlockComment;
                } else if is_string_quote(character, language) {
                    output.push(' ');
                    index += 1;
                    state = StripState::String {
                        quote: character,
                        escaped: false,
                    };
                } else {
                    output.push(character);
                    index += 1;
                }
            }
            StripState::LineComment => {
                if character == '\n' {
                    output.push('\n');
                    state = StripState::Normal;
                } else {
                    output.push(' ');
                }
                index += 1;
            }
            StripState::BlockComment => {
                if character == '*' && chars.get(index + 1) == Some(&'/') {
                    output.push(' ');
                    output.push(' ');
                    index += 2;
                    state = StripState::Normal;
                } else {
                    output.push(if character == '\n' { '\n' } else { ' ' });
                    index += 1;
                }
            }
            StripState::String { quote, escaped } => {
                output.push(if character == '\n' { '\n' } else { ' ' });
                index += 1;
                state = if escaped {
                    StripState::String {
                        quote,
                        escaped: false,
                    }
                } else if character == '\\' {
                    StripState::String {
                        quote,
                        escaped: true,
                    }
                } else if character == quote {
                    StripState::Normal
                } else {
                    StripState::String {
                        quote,
                        escaped: false,
                    }
                };
            }
        }
    }

    output
}

#[derive(Debug, Clone, Copy)]
enum StripState {
    Normal,
    LineComment,
    BlockComment,
    String { quote: char, escaped: bool },
}

fn line_comment_len(chars: &[char], index: usize, language: Language) -> Option<usize> {
    match language {
        Language::Python if chars[index] == '#' => Some(1),
        Language::Rust | Language::TypeScript | Language::JavaScript => {
            (chars[index] == '/' && chars.get(index + 1) == Some(&'/')).then_some(2)
        }
        Language::Markdown | Language::Json | Language::Toml | Language::Yaml | Language::Text => {
            None
        }
        _ => None,
    }
}

fn block_comment_len(chars: &[char], index: usize, language: Language) -> Option<usize> {
    (matches!(
        language,
        Language::Rust | Language::TypeScript | Language::JavaScript
    ) && chars[index] == '/'
        && chars.get(index + 1) == Some(&'*'))
    .then_some(2)
}

fn is_string_quote(character: char, language: Language) -> bool {
    match language {
        Language::TypeScript | Language::JavaScript => matches!(character, '\'' | '"' | '`'),
        Language::Python | Language::Rust => matches!(character, '\'' | '"'),
        Language::Markdown | Language::Json | Language::Toml | Language::Yaml | Language::Text => {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_calls_and_references_for_candidate_names() {
        let names = BTreeSet::from(["createSession".to_string(), "Session".to_string()]);
        let references = extract_references(
            "export function createSession() {}\nconst session = createSession(user);\ntype T = Session;\n",
            Language::TypeScript,
            &names,
        );

        assert!(references.iter().any(|reference| {
            reference.target_name == "createSession" && reference.kind == "call"
        }));
        assert!(
            references.iter().any(
                |reference| reference.target_name == "Session" && reference.kind == "reference"
            )
        );
        assert_eq!(
            references
                .iter()
                .filter(|reference| reference.target_name == "createSession")
                .count(),
            1
        );
    }

    #[test]
    fn ignores_comments_and_strings_for_code_references() {
        let names = BTreeSet::from(["createSession".to_string()]);
        let references = extract_references(
            "// createSession(user)\nconst text = 'createSession(user)';\ncreateSession(user);\n",
            Language::TypeScript,
            &names,
        );

        assert_eq!(references.len(), 1);
        assert_eq!(references[0].line, 3);
        assert_eq!(references[0].kind, "call");
    }
}
