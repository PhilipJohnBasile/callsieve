use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::WalkBuilder;

use super::language::Language;

const SKIPPED_DIRS: &[&str] = &[
    ".git",
    ".callsieve",
    "node_modules",
    "target",
    "dist",
    "build",
    "coverage",
    "vendor",
];

const SKIPPED_FILES: &[&str] = &[
    "cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "deno.lock",
];

pub fn source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in WalkBuilder::new(root)
        .standard_filters(true)
        .hidden(false)
        .require_git(false)
        .build()
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();

        if should_skip(path)
            || !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }

        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };

        if is_skipped_file(relative) {
            continue;
        }

        if Language::from_path(relative).is_some() {
            files.push(relative.to_path_buf());
        }
    }

    files.sort();
    Ok(files)
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| {
        let value = component.as_os_str().to_string_lossy();
        SKIPPED_DIRS
            .iter()
            .any(|skipped| value.eq_ignore_ascii_case(skipped))
    })
}

fn is_skipped_file(path: &Path) -> bool {
    if is_generated_trace_file(path) {
        return true;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    SKIPPED_FILES
        .iter()
        .any(|skipped| file_name.eq_ignore_ascii_case(skipped))
}

fn is_generated_trace_file(path: &Path) -> bool {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if !normalized.starts_with("benchmarks/") {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let file_name = file_name.to_ascii_lowercase();
    file_name.ends_with("-trace.json") || file_name.starts_with("session-trace")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn respects_gitignore_and_skips_internal_dirs() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(temp.path().join("kept.ts"), "export function kept() {}\n").unwrap();
        fs::write(temp.path().join("README.md"), "# Kept docs\n").unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kept\"\n",
        )
        .unwrap();
        fs::write(temp.path().join("Cargo.lock"), "# lock\n").unwrap();
        fs::write(
            temp.path().join("ignored.ts"),
            "export function ignored() {}\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join(".github/workflows")).unwrap();
        fs::write(temp.path().join(".github/workflows/ci.yml"), "name: CI\n").unwrap();
        fs::create_dir_all(temp.path().join(".callsieve")).unwrap();
        fs::write(
            temp.path().join(".callsieve/index.ts"),
            "function internal() {}\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("benchmarks")).unwrap();
        fs::write(
            temp.path().join("benchmarks/external-ripgrep-trace.json"),
            "{}\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("benchmarks/session-trace.example.json"),
            "{}\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("benchmarks/callsieve-real-repo.json"),
            "{}\n",
        )
        .unwrap();

        let files = source_files(temp.path()).unwrap();

        assert_eq!(
            files,
            vec![
                PathBuf::from(".github/workflows/ci.yml"),
                PathBuf::from("Cargo.toml"),
                PathBuf::from("README.md"),
                PathBuf::from("benchmarks/callsieve-real-repo.json"),
                PathBuf::from("kept.ts"),
            ]
        );
    }
}
