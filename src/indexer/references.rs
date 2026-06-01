use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct RawReference {
    pub target_name: String,
    pub kind: String,
    pub line: usize,
}

pub fn extract_references(content: &str, candidate_names: &BTreeSet<String>) -> Vec<RawReference> {
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
                    });
                }
            }
        }
    }

    references
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_calls_and_references_for_candidate_names() {
        let names = BTreeSet::from(["createSession".to_string(), "Session".to_string()]);
        let references = extract_references(
            "export function createSession() {}\nconst session = createSession(user);\ntype T = Session;\n",
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
}
