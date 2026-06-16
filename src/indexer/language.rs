use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    TypeScript,
    JavaScript,
    Python,
    Rust,
    C,
    Cpp,
    Java,
    CSharp,
    VisualBasic,
    ClassicVisualBasic,
    Sql,
    R,
    Delphi,
    Scratch,
    Php,
    Go,
    Ada,
    Fortran,
    Perl,
    Assembly,
    Matlab,
    ObjectiveC,
    Cobol,
    Sas,
    Julia,
    PlSql,
    TransactSql,
    Gml,
    Prolog,
    LabView,
    Ruby,
    ML,
    Lisp,
    Zig,
    Kotlin,
    Swift,
    VbScript,
    Abap,
    LadderLogic,
    Xpp,
    D,
    OCaml,
    Caml,
    Erlang,
    PowerShell,
    Cfml,
    Scala,
    Elixir,
    Haskell,
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
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);

        match extension.as_deref() {
            Some("ts") | Some("tsx") => Some(Self::TypeScript),
            Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => Some(Self::JavaScript),
            Some("py") | Some("pyw") => Some(Self::Python),
            Some("rs") => Some(Self::Rust),
            Some("c") => Some(Self::C),
            Some("cpp") | Some("cc") | Some("cxx") | Some("c++") | Some("hpp") | Some("hh")
            | Some("hxx") | Some("h++") => Some(Self::Cpp),
            Some("h") => Some(Self::C),
            Some("java") => Some(Self::Java),
            Some("cs") => Some(Self::CSharp),
            Some("vb") => Some(Self::VisualBasic),
            Some("bas") | Some("cls") | Some("frm") | Some("ctl") => Some(Self::ClassicVisualBasic),
            Some("sql") => Some(Self::Sql),
            Some("r") => Some(Self::R),
            Some("pas") | Some("pp") | Some("dpr") | Some("dfm") => Some(Self::Delphi),
            Some("sb") | Some("sb2") | Some("sb3") | Some("sprite2") | Some("sprite3") => {
                Some(Self::Scratch)
            }
            Some("php") | Some("phtml") | Some("php3") | Some("php4") | Some("php5") => {
                Some(Self::Php)
            }
            Some("go") => Some(Self::Go),
            Some("adb") | Some("ads") | Some("ada") => Some(Self::Ada),
            Some("f") | Some("for") | Some("ftn") | Some("f77") | Some("f90") | Some("f95")
            | Some("f03") | Some("f08") => Some(Self::Fortran),
            Some("pl") | Some("pm") | Some("pod") => Some(Self::Perl),
            Some("asm") | Some("s") | Some("inc") => Some(Self::Assembly),
            Some("m") | Some("mlx") => Some(Self::Matlab),
            Some("mm") | Some("objc") => Some(Self::ObjectiveC),
            Some("cob") | Some("cbl") | Some("cpy") | Some("cobol") => Some(Self::Cobol),
            Some("sas") => Some(Self::Sas),
            Some("jl") => Some(Self::Julia),
            Some("pls") | Some("pkb") | Some("pks") | Some("plsql") => Some(Self::PlSql),
            Some("tsql") => Some(Self::TransactSql),
            Some("gml") => Some(Self::Gml),
            Some("pro") | Some("prolog") => Some(Self::Prolog),
            Some("vi") | Some("vim") | Some("lvproj") | Some("lvlib") => Some(Self::LabView),
            Some("rb") | Some("rake") => Some(Self::Ruby),
            Some("sml") | Some("sig") | Some("fun") => Some(Self::ML),
            Some("lisp") | Some("lsp") | Some("cl") | Some("el") | Some("scm") => Some(Self::Lisp),
            Some("zig") => Some(Self::Zig),
            Some("kt") | Some("kts") => Some(Self::Kotlin),
            Some("swift") => Some(Self::Swift),
            Some("vbs") => Some(Self::VbScript),
            Some("abap") => Some(Self::Abap),
            Some("lad") | Some("ld") | Some("l5x") => Some(Self::LadderLogic),
            Some("xpp") => Some(Self::Xpp),
            Some("d") => Some(Self::D),
            Some("ml") | Some("mli") => Some(Self::OCaml),
            Some("caml") => Some(Self::Caml),
            Some("erl") | Some("hrl") => Some(Self::Erlang),
            Some("ps1") | Some("psm1") | Some("psd1") => Some(Self::PowerShell),
            Some("cfm") | Some("cfc") | Some("cfml") => Some(Self::Cfml),
            Some("scala") | Some("sc") => Some(Self::Scala),
            Some("ex") | Some("exs") => Some(Self::Elixir),
            Some("hs") | Some("lhs") => Some(Self::Haskell),
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

    pub fn from_path_with_content(path: &Path, content: &str) -> Option<Self> {
        let language = Self::from_path(path)?;
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);

        if matches!(extension.as_deref(), Some("m") | Some("h")) && looks_like_objective_c(content)
        {
            return Some(Self::ObjectiveC);
        }

        Some(language)
    }

    pub fn is_code(self) -> bool {
        !matches!(
            self,
            Self::Markdown | Self::Json | Self::Toml | Self::Yaml | Self::Text
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::VisualBasic => "visualbasic",
            Self::ClassicVisualBasic => "classicvisualbasic",
            Self::Sql => "sql",
            Self::R => "r",
            Self::Delphi => "delphi",
            Self::Scratch => "scratch",
            Self::Php => "php",
            Self::Go => "go",
            Self::Ada => "ada",
            Self::Fortran => "fortran",
            Self::Perl => "perl",
            Self::Assembly => "assembly",
            Self::Matlab => "matlab",
            Self::ObjectiveC => "objectivec",
            Self::Cobol => "cobol",
            Self::Sas => "sas",
            Self::Julia => "julia",
            Self::PlSql => "plsql",
            Self::TransactSql => "transactsql",
            Self::Gml => "gml",
            Self::Prolog => "prolog",
            Self::LabView => "labview",
            Self::Ruby => "ruby",
            Self::ML => "ml",
            Self::Lisp => "lisp",
            Self::Zig => "zig",
            Self::Kotlin => "kotlin",
            Self::Swift => "swift",
            Self::VbScript => "vbscript",
            Self::Abap => "abap",
            Self::LadderLogic => "ladderlogic",
            Self::Xpp => "xpp",
            Self::D => "d",
            Self::OCaml => "ocaml",
            Self::Caml => "caml",
            Self::Erlang => "erlang",
            Self::PowerShell => "powershell",
            Self::Cfml => "cfml",
            Self::Scala => "scala",
            Self::Elixir => "elixir",
            Self::Haskell => "haskell",
            Self::Dart => "dart",
            Self::Lua => "lua",
            Self::Shell => "shell",
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Yaml => "yaml",
            Self::Text => "text",
        }
    }
}

fn looks_like_objective_c(content: &str) -> bool {
    content.contains("@interface")
        || content.contains("@implementation")
        || content.contains("@protocol")
        || content.contains("#import <Foundation")
        || content.contains("#import \"")
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
        assert_eq!(Language::from_path(Path::new("a.c")), Some(Language::C));
        assert_eq!(Language::from_path(Path::new("a.cpp")), Some(Language::Cpp));
        assert_eq!(
            Language::from_path(Path::new("a.java")),
            Some(Language::Java)
        );
        assert_eq!(
            Language::from_path(Path::new("a.cs")),
            Some(Language::CSharp)
        );
        assert_eq!(
            Language::from_path(Path::new("a.vb")),
            Some(Language::VisualBasic)
        );
        assert_eq!(
            Language::from_path(Path::new("a.bas")),
            Some(Language::ClassicVisualBasic)
        );
        assert_eq!(Language::from_path(Path::new("a.sql")), Some(Language::Sql));
        assert_eq!(Language::from_path(Path::new("a.r")), Some(Language::R));
        assert_eq!(
            Language::from_path(Path::new("a.pas")),
            Some(Language::Delphi)
        );
        assert_eq!(
            Language::from_path(Path::new("a.sb3")),
            Some(Language::Scratch)
        );
        assert_eq!(Language::from_path(Path::new("a.php")), Some(Language::Php));
        assert_eq!(Language::from_path(Path::new("a.go")), Some(Language::Go));
        assert_eq!(Language::from_path(Path::new("a.adb")), Some(Language::Ada));
        assert_eq!(
            Language::from_path(Path::new("a.f90")),
            Some(Language::Fortran)
        );
        assert_eq!(Language::from_path(Path::new("a.pl")), Some(Language::Perl));
        assert_eq!(
            Language::from_path(Path::new("a.asm")),
            Some(Language::Assembly)
        );
        assert_eq!(
            Language::from_path(Path::new("a.m")),
            Some(Language::Matlab)
        );
        assert_eq!(
            Language::from_path_with_content(Path::new("a.m"), "@interface Thing\n@end"),
            Some(Language::ObjectiveC)
        );
        assert_eq!(
            Language::from_path(Path::new("a.cob")),
            Some(Language::Cobol)
        );
        assert_eq!(Language::from_path(Path::new("a.sas")), Some(Language::Sas));
        assert_eq!(
            Language::from_path(Path::new("a.jl")),
            Some(Language::Julia)
        );
        assert_eq!(
            Language::from_path(Path::new("a.plsql")),
            Some(Language::PlSql)
        );
        assert_eq!(
            Language::from_path(Path::new("a.tsql")),
            Some(Language::TransactSql)
        );
        assert_eq!(Language::from_path(Path::new("a.gml")), Some(Language::Gml));
        assert_eq!(
            Language::from_path(Path::new("a.prolog")),
            Some(Language::Prolog)
        );
        assert_eq!(
            Language::from_path(Path::new("a.lvproj")),
            Some(Language::LabView)
        );
        assert_eq!(Language::from_path(Path::new("a.rb")), Some(Language::Ruby));
        assert_eq!(Language::from_path(Path::new("a.sml")), Some(Language::ML));
        assert_eq!(
            Language::from_path(Path::new("a.lisp")),
            Some(Language::Lisp)
        );
        assert_eq!(Language::from_path(Path::new("a.zig")), Some(Language::Zig));
        assert_eq!(
            Language::from_path(Path::new("a.kt")),
            Some(Language::Kotlin)
        );
        assert_eq!(
            Language::from_path(Path::new("a.swift")),
            Some(Language::Swift)
        );
        assert_eq!(
            Language::from_path(Path::new("a.vbs")),
            Some(Language::VbScript)
        );
        assert_eq!(
            Language::from_path(Path::new("a.abap")),
            Some(Language::Abap)
        );
        assert_eq!(
            Language::from_path(Path::new("a.lad")),
            Some(Language::LadderLogic)
        );
        assert_eq!(Language::from_path(Path::new("a.xpp")), Some(Language::Xpp));
        assert_eq!(Language::from_path(Path::new("a.d")), Some(Language::D));
        assert_eq!(
            Language::from_path(Path::new("a.ml")),
            Some(Language::OCaml)
        );
        assert_eq!(
            Language::from_path(Path::new("a.caml")),
            Some(Language::Caml)
        );
        assert_eq!(
            Language::from_path(Path::new("a.erl")),
            Some(Language::Erlang)
        );
        assert_eq!(
            Language::from_path(Path::new("a.ps1")),
            Some(Language::PowerShell)
        );
        assert_eq!(
            Language::from_path(Path::new("a.cfc")),
            Some(Language::Cfml)
        );
        assert_eq!(
            Language::from_path(Path::new("a.scala")),
            Some(Language::Scala)
        );
        assert_eq!(
            Language::from_path(Path::new("a.ex")),
            Some(Language::Elixir)
        );
        assert_eq!(
            Language::from_path(Path::new("a.hs")),
            Some(Language::Haskell)
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
