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

    let mut symbols = match tree_sitter_symbols::extract_symbols(content, language) {
        Some(symbols) if !symbols.is_empty() => symbols,
        _ => extract_symbols_fallback(content, language),
    };
    if language == Language::Rust {
        mark_rust_test_functions(content, &mut symbols);
    }
    symbols
}

/// Mark `#[test]` / `#[bench]` / `#[cfg(test)]`-attributed functions (e.g. a bare `#[test] fn` not
/// inside a `mod tests`) as `test` kind, so test-aware ranking recognizes them as scaffolding rather
/// than implementation. Scans the attribute and blank lines directly above each function.
fn mark_rust_test_functions(content: &str, symbols: &mut [RawSymbol]) {
    let lines: Vec<&str> = content.lines().collect();
    for symbol in symbols.iter_mut() {
        if symbol.kind != "function" {
            continue;
        }
        let mut above = symbol.start_line;
        while above > 1 {
            above -= 1;
            let line = match lines.get(above - 1) {
                Some(line) => line.trim(),
                None => break,
            };
            if line.is_empty() {
                continue;
            }
            if line.starts_with("#[") {
                let lower = line.to_ascii_lowercase();
                if lower.contains("test") || lower.contains("bench") {
                    symbol.kind = "test".to_string();
                    break;
                }
                continue; // stacked attributes — keep scanning upward
            }
            break; // first non-attribute, non-blank line ends the attribute block
        }
    }
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
        Language::Sql | Language::PlSql | Language::TransactSql => parse_sql_line(line),
        Language::R => parse_r_line(line),
        Language::Java => parse_java_line(line),
        Language::CSharp => parse_csharp_line(line),
        Language::VisualBasic | Language::ClassicVisualBasic | Language::VbScript => {
            parse_basic_line(line)
        }
        Language::C => parse_c_line(line),
        Language::Cpp => parse_cpp_line(line),
        Language::Delphi => parse_delphi_line(line),
        Language::Scratch | Language::LabView => None,
        Language::Ada => parse_ada_line(line),
        Language::Fortran => parse_fortran_line(line),
        Language::Perl => parse_perl_line(line),
        Language::Assembly => parse_assembly_line(line),
        Language::Matlab => parse_matlab_line(line),
        Language::ObjectiveC => parse_objective_c_line(line),
        Language::Cobol => parse_cobol_line(line),
        Language::Sas => parse_sas_line(line),
        Language::Julia => parse_julia_line(line),
        Language::Gml => parse_gml_line(line),
        Language::Prolog => parse_prolog_line(line),
        Language::Ruby => parse_ruby_line(line),
        Language::ML | Language::OCaml | Language::Caml => parse_ml_line(line),
        Language::Lisp => parse_lisp_line(line),
        Language::Zig => parse_zig_line(line),
        Language::Kotlin => parse_kotlin_line(line),
        Language::Swift => parse_swift_line(line),
        Language::Abap => parse_abap_line(line),
        Language::LadderLogic => parse_ladder_line(line),
        Language::Xpp => parse_xpp_line(line),
        Language::D => parse_d_line(line),
        Language::Erlang => parse_erlang_line(line),
        Language::PowerShell => parse_powershell_line(line),
        Language::Cfml => parse_cfml_line(line),
        Language::Scala => parse_scala_line(line),
        Language::Elixir => parse_elixir_line(line),
        Language::Haskell => parse_haskell_line(line),
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

    if indentation(line) == 0
        && let Some(name) = parse_python_constant_assignment(rest)
    {
        return Some((name, "constant".to_string(), "public".to_string()));
    }

    None
}

fn parse_python_constant_assignment(line: &str) -> Option<String> {
    if line.starts_with('#') {
        return None;
    }
    let (left, _) = line.split_once('=')?;
    if left.contains("==") || left.contains("!=") || left.contains("<=") || left.contains(">=") {
        return None;
    }
    let name = left.split_once(':').map_or(left, |(name, _)| name).trim();
    (is_python_constant_name(name)).then(|| name.to_string())
}

fn is_python_constant_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    let mut saw_uppercase = true;
    for character in chars {
        if character.is_ascii_uppercase() {
            saw_uppercase = true;
        } else if !character.is_ascii_digit() && character != '_' {
            return false;
        }
    }
    saw_uppercase
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

fn parse_sql_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if rest.starts_with("--") {
        return None;
    }
    let lower = rest.to_ascii_lowercase();
    for (keyword, kind) in [
        ("function", "function"),
        ("procedure", "procedure"),
        ("package body", "package"),
        ("package", "package"),
        ("trigger", "trigger"),
        ("view", "view"),
        ("table", "table"),
        ("type", "type"),
    ] {
        for prefix in [
            format!("create or replace {keyword} "),
            format!("create {keyword} "),
            format!("alter {keyword} "),
        ] {
            if lower.starts_with(&prefix) {
                return Some((
                    clean_sql_name(&rest[prefix.len()..])?,
                    kind.to_string(),
                    "public".to_string(),
                ));
            }
        }
    }
    None
}

fn clean_sql_name(input: &str) -> Option<String> {
    let name = input
        .trim()
        .trim_start_matches('[')
        .chars()
        .take_while(|character| {
            !character.is_whitespace() && !matches!(*character, '(' | '[' | ']' | ';')
        })
        .collect::<String>();
    let name = name
        .trim_matches(['"', '`', '[', ']'])
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .trim_matches(['"', '`', '[', ']'])
        .to_string();
    (!name.is_empty()).then_some(name)
}

fn parse_r_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if rest.starts_with('#') {
        return None;
    }
    for marker in ["<- function", "= function"] {
        if let Some((name, _)) = rest.split_once(marker) {
            let name = name.trim();
            if is_identifier(name) {
                return Some((
                    name.to_string(),
                    "function".to_string(),
                    "public".to_string(),
                ));
            }
        }
    }
    if let Some(after_prefix) = rest.strip_prefix("setClass(")
        && let Some(name) = first_quoted_local(after_prefix)
    {
        return Some((name, "class".to_string(), "public".to_string()));
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

fn parse_basic_line(line: &str) -> Option<(String, String, String)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('\'') || trimmed.to_ascii_lowercase().starts_with("rem ") {
        return None;
    }
    let rest = strip_leading_modifiers_ci(
        trimmed,
        &[
            "public",
            "private",
            "protected",
            "friend",
            "shared",
            "static",
            "partial",
            "async",
            "overrides",
            "overridable",
            "notinheritable",
            "mustinherit",
        ],
    );
    for (prefix, kind) in [
        ("class ", "class"),
        ("module ", "module"),
        ("interface ", "interface"),
        ("enum ", "enum"),
        ("structure ", "struct"),
        ("sub ", "function"),
        ("function ", "function"),
        ("property ", "property"),
    ] {
        if let Some(after_prefix) = strip_prefix_ci(rest, prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                visibility_from_modifiers_ci(trimmed, "public").to_string(),
            ));
        }
    }
    None
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

fn parse_delphi_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    let lower = rest.to_ascii_lowercase();
    if let Some(after_prefix) = strip_prefix_ci(rest, "unit ") {
        return Some((
            take_identifier(after_prefix.trim_end_matches(';'))?,
            "module".to_string(),
            "public".to_string(),
        ));
    }
    if lower.starts_with("type ")
        && let Some((name, right)) = rest[5..].split_once('=')
        && right.to_ascii_lowercase().contains("class")
    {
        return Some((
            take_identifier(name.trim())?,
            "class".to_string(),
            "public".to_string(),
        ));
    }
    for (prefix, kind) in [
        ("procedure ", "procedure"),
        ("function ", "function"),
        ("constructor ", "function"),
        ("destructor ", "function"),
    ] {
        if let Some(after_prefix) = strip_prefix_ci(rest, prefix) {
            return Some((
                last_qualified_part(&take_qualified_identifier(after_prefix)?),
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_ada_line(line: &str) -> Option<(String, String, String)> {
    let rest = strip_leading_modifiers_ci(line.trim_start(), &["private", "limited", "generic"]);
    for (prefix, kind) in [
        ("package body ", "package"),
        ("package ", "package"),
        ("procedure ", "procedure"),
        ("function ", "function"),
        ("task body ", "task"),
        ("task ", "task"),
        ("type ", "type"),
    ] {
        if let Some(after_prefix) = strip_prefix_ci(rest, prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_fortran_line(line: &str) -> Option<(String, String, String)> {
    let rest = strip_leading_modifiers_ci(
        line.trim_start(),
        &["recursive", "pure", "elemental", "module"],
    );
    for (prefix, kind) in [
        ("program ", "program"),
        ("module ", "module"),
        ("subroutine ", "function"),
        ("function ", "function"),
        ("type ", "type"),
    ] {
        if let Some(after_prefix) = strip_prefix_ci(rest, prefix) {
            return Some((
                take_identifier(after_prefix.trim_start_matches("::").trim_start())?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_perl_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if let Some(after_prefix) = rest.strip_prefix("package ") {
        return Some((
            last_qualified_part(&take_qualified_identifier(after_prefix)?),
            "module".to_string(),
            "public".to_string(),
        ));
    }
    if let Some(after_prefix) = rest.strip_prefix("sub ") {
        return Some((
            take_identifier(after_prefix)?,
            "function".to_string(),
            "public".to_string(),
        ));
    }
    None
}

fn parse_assembly_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if rest.is_empty() || rest.starts_with([';', '#', '.']) {
        return None;
    }
    let (label, _) = rest.split_once(':')?;
    let label = label.trim();
    if label.is_empty() || label.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some((label.to_string(), "label".to_string(), "local".to_string()))
}

fn parse_matlab_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if let Some(after_prefix) = rest.strip_prefix("classdef ") {
        return Some((
            take_identifier(strip_leading_modifiers(
                after_prefix,
                &["abstract", "sealed"],
            ))?,
            "class".to_string(),
            "public".to_string(),
        ));
    }
    if let Some(after_prefix) = rest.strip_prefix("function ") {
        let after_output = after_prefix
            .split_once('=')
            .map(|(_, after)| after.trim_start())
            .unwrap_or(after_prefix);
        return Some((
            take_identifier(after_output)?,
            "function".to_string(),
            "public".to_string(),
        ));
    }
    None
}

fn parse_objective_c_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("@interface ", "class"),
        ("@implementation ", "class"),
        ("@protocol ", "interface"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    if (rest.starts_with("- ") || rest.starts_with("+ "))
        && let Some(close_paren) = rest.find(')')
    {
        let selector = rest[close_paren + 1..].trim_start();
        let name = selector
            .split([':', ';', '{', ' '])
            .next()
            .unwrap_or_default()
            .trim();
        if is_identifier(name) {
            return Some((name.to_string(), "method".to_string(), "public".to_string()));
        }
    }
    None
}

fn parse_cobol_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    let upper = rest.to_ascii_uppercase();
    if let Some(index) = upper.find("PROGRAM-ID.") {
        return Some((
            take_extended_identifier(rest[index + "PROGRAM-ID.".len()..].trim())?,
            "program".to_string(),
            "public".to_string(),
        ));
    }
    if upper.ends_with(" SECTION.") {
        return Some((
            take_extended_identifier(rest.trim_end_matches('.'))?,
            "section".to_string(),
            "public".to_string(),
        ));
    }
    None
}

fn parse_sas_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("%macro ", "macro"),
        ("data ", "dataset"),
        ("proc ", "procedure"),
    ] {
        if let Some(after_prefix) = strip_prefix_ci(rest, prefix) {
            return Some((
                take_extended_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_julia_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("function ", "function"),
        ("struct ", "struct"),
        ("mutable struct ", "struct"),
        ("module ", "module"),
        ("macro ", "macro"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    if let Some(name) = method_name_before_paren(rest)
        && rest.contains('=')
    {
        return Some((name, "function".to_string(), "public".to_string()));
    }
    None
}

fn parse_gml_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if let Some(after_prefix) = rest.strip_prefix("function ") {
        return Some((
            take_identifier(after_prefix)?,
            "function".to_string(),
            "public".to_string(),
        ));
    }
    parse_js_line(line)
}

fn parse_prolog_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if rest.starts_with('%') || rest.starts_with(":-") {
        return None;
    }
    if let Some(paren_index) = rest.find('(') {
        let name = &rest[..paren_index];
        if is_identifier(name) && (rest.contains(":-") || rest.contains('.')) {
            return Some((
                name.to_string(),
                "predicate".to_string(),
                "public".to_string(),
            ));
        }
    }
    None
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

fn parse_ml_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("module ", "module"),
        ("structure ", "module"),
        ("signature ", "interface"),
        ("functor ", "module"),
        ("let rec ", "function"),
        ("let ", "value"),
        ("fun ", "function"),
        ("val ", "value"),
        ("type ", "type"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_lisp_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start().trim_start_matches('(');
    for (prefix, kind) in [
        ("defun ", "function"),
        ("defmacro ", "macro"),
        ("defclass ", "class"),
        ("defstruct ", "struct"),
        ("defvar ", "variable"),
        ("defparameter ", "variable"),
        ("define ", "function"),
    ] {
        if let Some(after_prefix) = strip_prefix_ci(rest, prefix) {
            return Some((
                take_extended_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_zig_line(line: &str) -> Option<(String, String, String)> {
    let rest = strip_leading_modifiers(line.trim_start(), &["pub", "export", "extern", "inline"]);
    if let Some(after_prefix) = rest.strip_prefix("fn ") {
        return Some((
            take_identifier(after_prefix)?,
            "function".to_string(),
            visibility_from_modifiers(line.trim_start(), "private").to_string(),
        ));
    }
    if let Some(after_prefix) = rest.strip_prefix("const ")
        && let Some((name, right)) = after_prefix.split_once('=')
    {
        let kind = if right.contains("struct") {
            "struct"
        } else if right.contains("enum") {
            "enum"
        } else if right.contains("union") {
            "union"
        } else {
            "constant"
        };
        return Some((
            take_identifier(name.trim())?,
            kind.to_string(),
            visibility_from_modifiers(line.trim_start(), "private").to_string(),
        ));
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

fn parse_abap_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("CLASS ", "class"),
        ("METHOD ", "method"),
        ("FORM ", "function"),
        ("FUNCTION ", "function"),
        ("MODULE ", "module"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_extended_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_ladder_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("PROGRAM ", "program"),
        ("FUNCTION_BLOCK ", "function_block"),
        ("FUNCTION ", "function"),
        ("ROUTINE ", "routine"),
        ("TASK ", "task"),
    ] {
        if let Some(after_prefix) = strip_prefix_ci(rest, prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_xpp_line(line: &str) -> Option<(String, String, String)> {
    parse_c_style_line(
        line,
        &[
            "public",
            "private",
            "protected",
            "static",
            "final",
            "abstract",
            "server",
            "client",
        ],
        &[
            ("class ", "class"),
            ("interface ", "interface"),
            ("enum ", "enum"),
        ],
    )
}

fn parse_d_line(line: &str) -> Option<(String, String, String)> {
    parse_c_style_line(
        line,
        &[
            "public",
            "private",
            "protected",
            "static",
            "final",
            "abstract",
            "override",
            "extern",
            "immutable",
        ],
        &[
            ("class ", "class"),
            ("interface ", "interface"),
            ("struct ", "struct"),
            ("enum ", "enum"),
            ("template ", "template"),
        ],
    )
}

fn parse_erlang_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if let Some(after_prefix) = rest.strip_prefix("-module(")
        && let Some(name) = after_prefix.split_once(')').map(|(name, _)| name.trim())
    {
        return Some((name.to_string(), "module".to_string(), "public".to_string()));
    }
    if let Some(paren_index) = rest.find('(') {
        let name = &rest[..paren_index];
        if is_identifier(name) && rest.contains("->") {
            return Some((
                name.to_string(),
                "function".to_string(),
                "public".to_string(),
            ));
        }
    }
    None
}

fn parse_powershell_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if let Some(after_prefix) = strip_prefix_ci(rest, "function ") {
        return Some((
            take_extended_identifier(after_prefix)?,
            "function".to_string(),
            "public".to_string(),
        ));
    }
    if let Some(after_prefix) = strip_prefix_ci(rest, "class ") {
        return Some((
            take_identifier(after_prefix)?,
            "class".to_string(),
            "public".to_string(),
        ));
    }
    None
}

fn parse_cfml_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    if let Some(after_prefix) = rest.strip_prefix("component ") {
        let name =
            take_attribute_value(after_prefix, "name").or_else(|| take_identifier(after_prefix));
        return name.map(|name| (name, "component".to_string(), "public".to_string()));
    }
    if let Some(after_prefix) = strip_prefix_ci(rest, "function ") {
        return Some((
            take_identifier(after_prefix)?,
            "function".to_string(),
            "public".to_string(),
        ));
    }
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("<cffunction")
        && let Some(name) = take_attribute_value(rest, "name")
    {
        return Some((name, "function".to_string(), "public".to_string()));
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

fn parse_elixir_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind, visibility) in [
        ("defmodule ", "module", "public"),
        ("defprotocol ", "interface", "public"),
        ("defimpl ", "impl", "public"),
        ("defmacro ", "macro", "public"),
        ("defmacrop ", "macro", "private"),
        ("defp ", "function", "private"),
        ("def ", "function", "public"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_qualified_identifier(after_prefix)?,
                kind.to_string(),
                visibility.to_string(),
            ));
        }
    }
    None
}

fn parse_haskell_line(line: &str) -> Option<(String, String, String)> {
    let rest = line.trim_start();
    for (prefix, kind) in [
        ("module ", "module"),
        ("data ", "type"),
        ("newtype ", "type"),
        ("type ", "type"),
        ("class ", "class"),
    ] {
        if let Some(after_prefix) = rest.strip_prefix(prefix) {
            return Some((
                take_identifier(after_prefix)?,
                kind.to_string(),
                "public".to_string(),
            ));
        }
    }
    if let Some((name, _)) = rest.split_once("::") {
        let name = name.trim();
        if is_identifier(name) {
            return Some((
                name.to_string(),
                "function".to_string(),
                "public".to_string(),
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

fn strip_prefix_ci<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    input
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .then(|| &input[prefix.len()..])
}

fn strip_leading_modifiers_ci<'a>(mut input: &'a str, modifiers: &[&str]) -> &'a str {
    loop {
        let mut stripped = false;
        for modifier in modifiers {
            if let Some(rest) = strip_prefix_ci(input, modifier)
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

fn visibility_from_modifiers_ci(line: &str, default_visibility: &'static str) -> &'static str {
    for part in line.split_whitespace() {
        if part.eq_ignore_ascii_case("public") {
            return "public";
        }
        if part.eq_ignore_ascii_case("private") {
            return "private";
        }
        if part.eq_ignore_ascii_case("protected") {
            return "protected";
        }
        if part.eq_ignore_ascii_case("friend") || part.eq_ignore_ascii_case("internal") {
            return "internal";
        }
    }
    default_visibility
}

fn take_extended_identifier(input: &str) -> Option<String> {
    let identifier: String = input
        .trim_start()
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric()
                || matches!(*character, '_' | '-' | '.' | ':' | '$' | '#')
        })
        .collect();
    let identifier = identifier
        .trim_end_matches(['.', ';', ':'])
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .to_string();
    (!identifier.is_empty()).then_some(identifier)
}

fn first_quoted_local(input: &str) -> Option<String> {
    let quote_index = input.find(['\'', '"'])?;
    let quote = input.as_bytes()[quote_index] as char;
    let rest = &input[quote_index + 1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn take_attribute_value(input: &str, name: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let marker = format!("{}=", name.to_ascii_lowercase());
    let start = lower.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    let quote = rest.chars().next()?;
    if matches!(quote, '\'' | '"') {
        let value = &rest[quote.len_utf8()..];
        let end = value.find(quote)?;
        return Some(value[..end].to_string());
    }
    Some(
        rest.chars()
            .take_while(|character| !character.is_whitespace() && *character != '>')
            .collect(),
    )
    .filter(|value: &String| !value.is_empty())
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
    if language == Language::Python {
        return estimate_python_end_line(lines, start_index);
    }

    if uses_curly_end_estimate(language) {
        return estimate_curly_end_line(lines, start_index);
    }

    start_index + 1
}

fn uses_curly_end_estimate(language: Language) -> bool {
    matches!(
        language,
        Language::Rust
            | Language::TypeScript
            | Language::JavaScript
            | Language::Php
            | Language::Go
            | Language::Java
            | Language::CSharp
            | Language::C
            | Language::Cpp
            | Language::ObjectiveC
            | Language::Kotlin
            | Language::Swift
            | Language::Scala
            | Language::Dart
            | Language::R
            | Language::Julia
            | Language::Gml
            | Language::Zig
            | Language::Xpp
            | Language::D
            | Language::PowerShell
            | Language::Cfml
    )
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
    let doc = if language == Language::Rust && previous.starts_with("///") {
        previous.trim_start_matches("///")
    } else if hash_doc_language(language) && previous.starts_with('#') {
        previous.trim_start_matches('#')
    } else if slash_doc_language(language)
        && (previous.starts_with("//") || previous.starts_with('*'))
    {
        previous.trim_start_matches('/').trim_start_matches('*')
    } else if dash_doc_language(language) && previous.starts_with("--") {
        previous.trim_start_matches("--")
    } else if percent_doc_language(language) && previous.starts_with('%') {
        previous.trim_start_matches('%')
    } else if semicolon_doc_language(language) && previous.starts_with(';') {
        previous.trim_start_matches(';')
    } else if basic_doc_language(language) && previous.starts_with('\'') {
        previous.trim_start_matches('\'')
    } else {
        return None;
    };

    let doc = doc.trim();
    (!doc.is_empty()).then(|| doc.to_string())
}

fn hash_doc_language(language: Language) -> bool {
    matches!(
        language,
        Language::Python
            | Language::Ruby
            | Language::Shell
            | Language::R
            | Language::Perl
            | Language::Julia
            | Language::PowerShell
            | Language::Elixir
    )
}

fn slash_doc_language(language: Language) -> bool {
    matches!(
        language,
        Language::TypeScript
            | Language::JavaScript
            | Language::Php
            | Language::Go
            | Language::Java
            | Language::CSharp
            | Language::C
            | Language::Cpp
            | Language::ObjectiveC
            | Language::Kotlin
            | Language::Swift
            | Language::Scala
            | Language::Dart
            | Language::Lua
            | Language::Gml
            | Language::Zig
            | Language::Xpp
            | Language::D
            | Language::Cfml
    )
}

fn dash_doc_language(language: Language) -> bool {
    matches!(
        language,
        Language::Sql
            | Language::PlSql
            | Language::TransactSql
            | Language::Ada
            | Language::Haskell
            | Language::Lua
    )
}

fn percent_doc_language(language: Language) -> bool {
    matches!(
        language,
        Language::Matlab | Language::Erlang | Language::Prolog
    )
}

fn semicolon_doc_language(language: Language) -> bool {
    matches!(language, Language::Assembly | Language::Lisp)
}

fn basic_doc_language(language: Language) -> bool {
    matches!(
        language,
        Language::VisualBasic | Language::ClassicVisualBasic | Language::VbScript
    )
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
    fn bare_test_attribute_marks_function_as_test_kind() {
        let symbols = extract_symbols(
            "pub fn real() -> u32 {\n    1\n}\n\n#[test]\nfn checks_real() {\n    assert_eq!(real(), 1);\n}\n",
            Language::Rust,
        );
        let kind_of = |name: &str| {
            symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .map(|symbol| symbol.kind.as_str())
        };
        assert_eq!(kind_of("real"), Some("function"));
        assert_eq!(kind_of("checks_real"), Some("test"));
    }

    #[test]
    fn python_fallback_detects_top_level_constants() {
        assert_eq!(
            parse_python_line("FILE_UPLOAD_PERMISSIONS = None"),
            Some((
                "FILE_UPLOAD_PERMISSIONS".to_string(),
                "constant".to_string(),
                "public".to_string()
            ))
        );
        assert!(parse_python_line("lower_value = 1").is_none());
        assert!(parse_python_line("    LOCAL_SETTING = True").is_none());
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
