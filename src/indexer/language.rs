use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    TypeScript,
    JavaScript,
    Python,
    Rust,
    Php,
    Go,
    Java,
    CSharp,
    C,
    Cpp,
    Ruby,
    Kotlin,
    Swift,
    Scala,
    Dart,
    Lua,
    Shell,
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
            Some("php") | Some("phtml") => Some(Self::Php),
            Some("go") => Some(Self::Go),
            Some("java") => Some(Self::Java),
            Some("cs") => Some(Self::CSharp),
            Some("c") | Some("h") => Some(Self::C),
            Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") | Some("hh") | Some("hxx") => {
                Some(Self::Cpp)
            }
            Some("rb") => Some(Self::Ruby),
            Some("kt") | Some("kts") => Some(Self::Kotlin),
            Some("swift") => Some(Self::Swift),
            Some("scala") | Some("sc") => Some(Self::Scala),
            Some("dart") => Some(Self::Dart),
            Some("lua") => Some(Self::Lua),
            Some("sh") | Some("bash") | Some("zsh") | Some("fish") => Some(Self::Shell),
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
            Self::TypeScript
                | Self::JavaScript
                | Self::Python
                | Self::Rust
                | Self::Php
                | Self::Go
                | Self::Java
                | Self::CSharp
                | Self::C
                | Self::Cpp
                | Self::Ruby
                | Self::Kotlin
                | Self::Swift
                | Self::Scala
                | Self::Dart
                | Self::Lua
                | Self::Shell
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
        assert_eq!(Language::from_path(Path::new("a.php")), Some(Language::Php));
        assert_eq!(Language::from_path(Path::new("a.go")), Some(Language::Go));
        assert_eq!(
            Language::from_path(Path::new("a.java")),
            Some(Language::Java)
        );
        assert_eq!(
            Language::from_path(Path::new("a.cs")),
            Some(Language::CSharp)
        );
        assert_eq!(Language::from_path(Path::new("a.c")), Some(Language::C));
        assert_eq!(Language::from_path(Path::new("a.cpp")), Some(Language::Cpp));
        assert_eq!(Language::from_path(Path::new("a.rb")), Some(Language::Ruby));
        assert_eq!(
            Language::from_path(Path::new("a.kt")),
            Some(Language::Kotlin)
        );
        assert_eq!(
            Language::from_path(Path::new("a.swift")),
            Some(Language::Swift)
        );
        assert_eq!(
            Language::from_path(Path::new("a.scala")),
            Some(Language::Scala)
        );
        assert_eq!(
            Language::from_path(Path::new("a.dart")),
            Some(Language::Dart)
        );
        assert_eq!(Language::from_path(Path::new("a.lua")), Some(Language::Lua));
        assert_eq!(
            Language::from_path(Path::new("a.sh")),
            Some(Language::Shell)
        );
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
