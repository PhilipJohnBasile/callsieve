use super::language::Language;

#[derive(Debug, Clone)]
pub struct RawImport {
    pub imported: String,
}

pub fn extract_imports(content: &str, language: Language) -> Vec<RawImport> {
    let mut imports = Vec::new();

    for line in content.lines() {
        match language {
            Language::Rust => extract_rust_import(line, &mut imports),
            Language::Python => extract_python_import(line, &mut imports),
            Language::TypeScript | Language::JavaScript => extract_js_import(line, &mut imports),
        }
    }

    imports
}

fn extract_rust_import(line: &str, imports: &mut Vec<RawImport>) {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("use ") {
        imports.push(RawImport {
            imported: rest.trim_end_matches(';').trim().to_string(),
        });
    }
}

fn extract_python_import(line: &str, imports: &mut Vec<RawImport>) {
    let trimmed = line.trim_start();

    if let Some(rest) = trimmed.strip_prefix("from ") {
        if let Some((module, _)) = rest.split_once(" import ") {
            imports.push(RawImport {
                imported: module.trim().to_string(),
            });
        }
        return;
    }

    if let Some(rest) = trimmed.strip_prefix("import ") {
        for module in rest.split(',') {
            let module = module.split_whitespace().next().unwrap_or_default();
            if !module.is_empty() {
                imports.push(RawImport {
                    imported: module.to_string(),
                });
            }
        }
    }
}

fn extract_js_import(line: &str, imports: &mut Vec<RawImport>) {
    let trimmed = line.trim_start();

    if trimmed.starts_with("import ") {
        if let Some(imported) = quoted_after(trimmed, " from ") {
            imports.push(RawImport { imported });
            return;
        }

        if let Some(imported) = first_quoted(trimmed) {
            imports.push(RawImport { imported });
            return;
        }
    }

    if let Some(require_index) = trimmed.find("require(")
        && let Some(imported) = first_quoted(&trimmed[require_index..])
    {
        imports.push(RawImport { imported });
    }
}

fn quoted_after(line: &str, marker: &str) -> Option<String> {
    let marker_index = line.find(marker)?;
    first_quoted(&line[marker_index + marker.len()..])
}

fn first_quoted(input: &str) -> Option<String> {
    let quote_index = input.find(['\'', '"'])?;
    let quote = input.as_bytes()[quote_index] as char;
    let rest = &input[quote_index + 1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_imports() {
        let js = extract_imports(
            "import { token } from './token';\nconst x = require(\"./x\");\n",
            Language::TypeScript,
        );
        assert_eq!(js.len(), 2);

        let python = extract_imports(
            "import os, sys\nfrom app.auth import session\n",
            Language::Python,
        );
        assert_eq!(python.len(), 3);

        let rust = extract_imports("use crate::auth::Session;\n", Language::Rust);
        assert_eq!(rust.len(), 1);
    }
}
