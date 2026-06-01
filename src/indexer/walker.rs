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

pub fn source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in WalkBuilder::new(root)
        .standard_filters(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn respects_gitignore_and_skips_internal_dirs() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(".gitignore"), "ignored.ts\n").unwrap();
        fs::write(temp.path().join("kept.ts"), "export function kept() {}\n").unwrap();
        fs::write(
            temp.path().join("ignored.ts"),
            "export function ignored() {}\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join(".callsieve")).unwrap();
        fs::write(
            temp.path().join(".callsieve/index.ts"),
            "function internal() {}\n",
        )
        .unwrap();

        let files = source_files(temp.path()).unwrap();

        assert_eq!(files, vec![PathBuf::from("kept.ts")]);
    }
}
