use super::language::Language;

#[derive(Debug, Clone)]
pub struct RawImport {
    pub imported: String,
    pub aliases: Vec<RawImportAlias>,
}

#[derive(Debug, Clone)]
pub struct RawImportAlias {
    pub local: String,
    pub imported: String,
}

pub fn extract_imports(content: &str, language: Language) -> Vec<RawImport> {
    let mut imports = Vec::new();

    for line in content.lines() {
        match language {
            Language::Rust => extract_rust_import(line, &mut imports),
            Language::Python => extract_python_import(line, &mut imports),
            Language::TypeScript | Language::JavaScript => extract_js_import(line, &mut imports),
            Language::Markdown
            | Language::Json
            | Language::Toml
            | Language::Yaml
            | Language::Text => {}
        }
    }

    imports
}

fn extract_rust_import(line: &str, imports: &mut Vec<RawImport>) {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("use ") {
        let imported = rest.trim_end_matches(';').trim().to_string();
        let aliases = rust_aliases(&imported);
        imports.push(RawImport { imported, aliases });
    }
}

fn extract_python_import(line: &str, imports: &mut Vec<RawImport>) {
    let trimmed = line.trim_start();

    if let Some(rest) = trimmed.strip_prefix("from ") {
        if let Some((module, names)) = rest.split_once(" import ") {
            imports.push(RawImport {
                imported: module.trim().to_string(),
                aliases: python_from_aliases(names),
            });
        }
        return;
    }

    if let Some(rest) = trimmed.strip_prefix("import ") {
        for module in rest.split(',') {
            let (imported, local) = python_import_alias(module);
            if !imported.is_empty() {
                imports.push(RawImport {
                    imported: imported.clone(),
                    aliases: local
                        .map(|local| {
                            vec![RawImportAlias {
                                local,
                                imported: imported.clone(),
                            }]
                        })
                        .unwrap_or_default(),
                });
            }
        }
    }
}

fn extract_js_import(line: &str, imports: &mut Vec<RawImport>) {
    let trimmed = line.trim_start();

    if trimmed.starts_with("import ") {
        if let Some(imported) = quoted_after(trimmed, " from ") {
            imports.push(RawImport {
                aliases: js_import_aliases(trimmed),
                imported,
            });
            return;
        }

        if let Some(imported) = first_quoted(trimmed) {
            imports.push(RawImport {
                imported,
                aliases: Vec::new(),
            });
            return;
        }
    }

    if let Some(require_index) = trimmed.find("require(")
        && let Some(imported) = first_quoted(&trimmed[require_index..])
    {
        imports.push(RawImport {
            imported,
            aliases: js_require_aliases(trimmed),
        });
    }
}

fn js_import_aliases(line: &str) -> Vec<RawImportAlias> {
    let Some((before_from, _)) = line.split_once(" from ") else {
        return Vec::new();
    };
    let mut specifier = before_from
        .trim_start()
        .strip_prefix("import ")
        .unwrap_or(before_from)
        .trim()
        .trim_end_matches(';')
        .trim()
        .to_string();

    if let Some(type_stripped) = specifier.strip_prefix("type ") {
        specifier = type_stripped.trim_start().to_string();
    }

    let mut aliases = Vec::new();
    if let Some((default_part, rest)) = specifier.split_once(',') {
        add_default_js_alias(default_part, &mut aliases);
        add_js_named_or_namespace_aliases(rest, &mut aliases);
    } else {
        add_js_named_or_namespace_aliases(&specifier, &mut aliases);
        if aliases.is_empty() {
            add_default_js_alias(&specifier, &mut aliases);
        }
    }

    aliases
}

fn add_default_js_alias(input: &str, aliases: &mut Vec<RawImportAlias>) {
    let local = input.trim();
    if is_identifier(local) {
        aliases.push(RawImportAlias {
            local: local.to_string(),
            imported: local.to_string(),
        });
    }
}

fn add_js_named_or_namespace_aliases(input: &str, aliases: &mut Vec<RawImportAlias>) {
    let input = input.trim();
    if input.starts_with('{') {
        let Some(end) = input.rfind('}') else {
            return;
        };
        for entry in input[1..end].split(',') {
            let entry = entry.trim().trim_start_matches("type ").trim();
            if entry.is_empty() {
                continue;
            }
            let (imported, local) = alias_pair(entry);
            if is_identifier(&imported) && is_identifier(&local) {
                aliases.push(RawImportAlias { local, imported });
            }
        }
    } else if let Some(namespace) = input.strip_prefix("* as ") {
        let local = namespace.trim();
        if is_identifier(local) {
            aliases.push(RawImportAlias {
                local: local.to_string(),
                imported: "*".to_string(),
            });
        }
    }
}

fn js_require_aliases(line: &str) -> Vec<RawImportAlias> {
    let Some((left, _)) = line.split_once("require(") else {
        return Vec::new();
    };
    let left = left.trim();
    if let Some((name, _)) = left.rsplit_once('=') {
        let local = name
            .trim()
            .trim_start_matches("const ")
            .trim_start_matches("let ")
            .trim_start_matches("var ")
            .trim();
        if is_identifier(local) {
            return vec![RawImportAlias {
                local: local.to_string(),
                imported: local.to_string(),
            }];
        }
    }
    Vec::new()
}

fn python_from_aliases(names: &str) -> Vec<RawImportAlias> {
    names
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() || entry == "*" {
                return None;
            }
            let (imported, local) = alias_pair(entry);
            Some(RawImportAlias { local, imported })
        })
        .collect()
}

fn python_import_alias(module: &str) -> (String, Option<String>) {
    let module = module.trim();
    if module.is_empty() {
        return (String::new(), None);
    }
    if let Some((imported, local)) = module.split_once(" as ") {
        return (imported.trim().to_string(), Some(local.trim().to_string()));
    }
    let imported = module
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    (imported, None)
}

fn rust_aliases(imported: &str) -> Vec<RawImportAlias> {
    if let Some((prefix, grouped)) = imported.split_once('{') {
        let Some(grouped) = grouped.rsplit_once('}').map(|(items, _)| items) else {
            return Vec::new();
        };
        let prefix = prefix.trim_end_matches("::");
        return grouped
            .split(',')
            .filter_map(|entry| {
                let entry = entry.trim();
                if entry.is_empty() {
                    return None;
                }
                let (imported_name, local) = alias_pair(entry);
                Some(RawImportAlias {
                    local,
                    imported: format!("{prefix}::{imported_name}"),
                })
            })
            .collect();
    }

    if let Some((path, local)) = imported.split_once(" as ") {
        return vec![RawImportAlias {
            local: local.trim().to_string(),
            imported: path.trim().to_string(),
        }];
    }

    imported
        .rsplit("::")
        .next()
        .filter(|name| is_identifier(name))
        .map(|name| {
            vec![RawImportAlias {
                local: name.to_string(),
                imported: imported.to_string(),
            }]
        })
        .unwrap_or_default()
}

fn alias_pair(entry: &str) -> (String, String) {
    if let Some((imported, local)) = entry.split_once(" as ") {
        (imported.trim().to_string(), local.trim().to_string())
    } else {
        let name = entry.trim().to_string();
        (name.clone(), name)
    }
}

fn is_identifier(input: &str) -> bool {
    let mut chars = input.chars();
    chars.next().is_some_and(|character| {
        character.is_ascii_alphabetic() || character == '_' || character == '$'
    }) && chars
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '$')
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
