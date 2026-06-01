use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    TypeScript,
    JavaScript,
    Python,
    Rust,
    Markdown,
    Json,
    Toml,
    Yaml,
    Text,
}

impl Language {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("ts") | Some("tsx") => Some(Self::TypeScript),
            Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => Some(Self::JavaScript),
            Some("py") => Some(Self::Python),
            Some("rs") => Some(Self::Rust),
            Some("md") | Some("mdx") => Some(Self::Markdown),
            Some("json") | Some("jsonc") => Some(Self::Json),
            Some("toml") => Some(Self::Toml),
            Some("yaml") | Some("yml") => Some(Self::Yaml),
            Some("txt") => Some(Self::Text),
            _ => None,
        }
    }

    pub fn is_code(self) -> bool {
        matches!(
            self,
            Self::TypeScript | Self::JavaScript | Self::Python | Self::Rust
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_languages() {
        assert_eq!(
            Language::from_path(Path::new("a.ts")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            Language::from_path(Path::new("a.tsx")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            Language::from_path(Path::new("a.js")),
            Some(Language::JavaScript)
        );
        assert_eq!(
            Language::from_path(Path::new("a.jsx")),
            Some(Language::JavaScript)
        );
        assert_eq!(
            Language::from_path(Path::new("a.py")),
            Some(Language::Python)
        );
        assert_eq!(Language::from_path(Path::new("a.rs")), Some(Language::Rust));
        assert_eq!(
            Language::from_path(Path::new("a.md")),
            Some(Language::Markdown)
        );
        assert_eq!(
            Language::from_path(Path::new("a.toml")),
            Some(Language::Toml)
        );
        assert_eq!(
            Language::from_path(Path::new("a.yml")),
            Some(Language::Yaml)
        );
        assert_eq!(
            Language::from_path(Path::new("a.json")),
            Some(Language::Json)
        );
        assert_eq!(Language::from_path(Path::new("a.lock")), None);
    }
}
