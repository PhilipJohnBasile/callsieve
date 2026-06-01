use tree_sitter::{Node, Parser};

use super::{language::Language, symbols::RawSymbol};

pub fn extract_symbols(content: &str, language: Language) -> Option<Vec<RawSymbol>> {
    let parser_language = parser_language(language)?;
    let mut parser = Parser::new();
    parser.set_language(&parser_language).ok()?;
    let tree = parser.parse(content, None)?;
    if tree.root_node().has_error() {
        return None;
    }

    let mut symbols = Vec::new();
    collect_symbols(tree.root_node(), content, language, &mut symbols);
    assign_parents(&mut symbols);
    Some(symbols)
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

fn collect_symbols(
    node: Node<'_>,
    content: &str,
    language: Language,
    symbols: &mut Vec<RawSymbol>,
) {
    if let Some(symbol) = symbol_for_node(node, content, language) {
        symbols.push(symbol);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbols(child, content, language, symbols);
    }
}

fn symbol_for_node(node: Node<'_>, content: &str, language: Language) -> Option<RawSymbol> {
    match language {
        Language::TypeScript | Language::JavaScript => js_symbol_for_node(node, content),
        Language::Python => python_symbol_for_node(node, content),
        Language::Rust => rust_symbol_for_node(node, content),
        Language::Markdown | Language::Json | Language::Toml | Language::Yaml | Language::Text => {
            None
        }
    }
}

fn js_symbol_for_node(node: Node<'_>, content: &str) -> Option<RawSymbol> {
    let kind = match node.kind() {
        "function_declaration" | "generator_function_declaration" => "function",
        "class_declaration" => "class",
        "method_definition" => "method",
        "interface_declaration" => "interface",
        "type_alias_declaration" => "type",
        "variable_declarator" => variable_kind(node, content)?,
        _ => return None,
    };
    let name = node_name(node, content).or_else(|| variable_name(node, content))?;
    Some(raw_symbol(
        node,
        content,
        name,
        kind,
        if is_exported(node, content) {
            "exported"
        } else {
            "local"
        },
    ))
}

fn python_symbol_for_node(node: Node<'_>, content: &str) -> Option<RawSymbol> {
    let kind = match node.kind() {
        "function_definition" => "function",
        "class_definition" => "class",
        _ => return None,
    };
    let name = node_name(node, content)?;
    let visibility = if name.starts_with('_') {
        "private"
    } else {
        "public"
    };
    Some(raw_symbol(node, content, name, kind, visibility))
}

fn rust_symbol_for_node(node: Node<'_>, content: &str) -> Option<RawSymbol> {
    let kind = match node.kind() {
        "function_item" => "function",
        "struct_item" => "struct",
        "enum_item" => "enum",
        "trait_item" => "trait",
        "impl_item" => "impl",
        "const_item" => "constant",
        _ => return None,
    };
    let name = node_name(node, content).or_else(|| rust_impl_name(node, content))?;
    Some(raw_symbol(
        node,
        content,
        name,
        kind,
        if rust_is_public(node, content) {
            "public"
        } else {
            "private"
        },
    ))
}

fn raw_symbol(
    node: Node<'_>,
    content: &str,
    name: String,
    kind: &str,
    visibility: &str,
) -> RawSymbol {
    RawSymbol {
        name,
        kind: kind.to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row.max(node.start_position().row) + 1,
        visibility: visibility.to_string(),
        parent: None,
        signature: compact_signature(line_at(content, node.start_position().row)),
        doc: None,
    }
}

fn node_name(node: Node<'_>, content: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|name| node_text(name, content))
        .map(clean_name)
        .filter(|name| !name.is_empty())
}

fn variable_name(node: Node<'_>, content: &str) -> Option<String> {
    node.child_by_field_name("pattern")
        .or_else(|| node.child_by_field_name("name"))
        .and_then(|name| node_text(name, content))
        .map(clean_name)
        .filter(|name| !name.is_empty())
}

fn variable_kind(node: Node<'_>, content: &str) -> Option<&'static str> {
    let value = node.child_by_field_name("value")?;
    Some(match value.kind() {
        "arrow_function" | "function" | "function_expression" => "function",
        _ => {
            let text = node_text(value, content).unwrap_or_default();
            if text.contains("=>") || text.contains("function") {
                "function"
            } else {
                "constant"
            }
        }
    })
}

fn is_exported(node: Node<'_>, content: &str) -> bool {
    let mut current = Some(node);
    while let Some(node) = current {
        if node.kind() == "export_statement" {
            return true;
        }
        current = node.parent();
    }

    line_at(content, node.start_position().row)
        .trim_start()
        .starts_with("export ")
}

fn rust_is_public(node: Node<'_>, content: &str) -> bool {
    node.children(&mut node.walk()).any(|child| {
        child.kind() == "visibility_modifier"
            && node_text(child, content).is_some_and(|text| text.starts_with("pub"))
    }) || line_at(content, node.start_position().row)
        .trim_start()
        .starts_with("pub ")
}

fn rust_impl_name(node: Node<'_>, content: &str) -> Option<String> {
    let text = node_text(node, content)?;
    let rest = text.trim_start().strip_prefix("impl")?.trim_start();
    let rest = rest.strip_prefix('<').map_or(rest, |after| {
        after
            .split_once('>')
            .map(|(_, after_generics)| after_generics.trim_start())
            .unwrap_or(after)
    });
    let after_trait = rest
        .rsplit_once(" for ")
        .map(|(_, type_name)| type_name)
        .unwrap_or(rest);
    let identifier = clean_name(after_trait);
    (!identifier.is_empty()).then_some(identifier)
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

fn node_text(node: Node<'_>, content: &str) -> Option<String> {
    node.utf8_text(content.as_bytes()).ok().map(str::to_string)
}

fn clean_name(input: impl AsRef<str>) -> String {
    input
        .as_ref()
        .trim()
        .trim_start_matches('#')
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
        .collect()
}

fn line_at(content: &str, zero_based_line: usize) -> &str {
    content.lines().nth(zero_based_line).unwrap_or_default()
}

fn compact_signature(line: &str) -> String {
    let signature = line.trim();
    if signature.len() > 160 {
        format!("{}...", &signature[..157])
    } else {
        signature.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typescript_class_methods_and_exports() {
        let symbols = extract_symbols(
            "export class Session {\n  renew() { return tokenFor(); }\n}\nexport const tokenFor = () => 'x';\n",
            Language::TypeScript,
        )
        .unwrap();

        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "Session" && symbol.visibility == "exported")
        );
        assert!(symbols.iter().any(|symbol| symbol.name == "renew"
            && symbol.kind == "method"
            && symbol.parent.as_deref() == Some("Session")));
        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "tokenFor" && symbol.kind == "function")
        );
    }

    #[test]
    fn parses_python_nested_methods() {
        let symbols = extract_symbols(
            "class Service:\n    def login(self):\n        return True\n",
            Language::Python,
        )
        .unwrap();

        assert!(
            symbols
                .iter()
                .any(|symbol| symbol.name == "login"
                    && symbol.parent.as_deref() == Some("Service"))
        );
    }

    #[test]
    fn parses_rust_impl_methods() {
        let symbols = extract_symbols(
            "pub struct User;\nimpl User {\n  pub fn name(&self) -> String { String::new() }\n}\n",
            Language::Rust,
        )
        .unwrap();

        assert!(symbols.iter().any(|symbol| symbol.name == "name"
            && symbol.parent.as_deref() == Some("User")
            && symbol.visibility == "public"));
    }
}
