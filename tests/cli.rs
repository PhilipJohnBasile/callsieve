use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use serde_json::Value;

fn callsieve() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_callsieve"))
}

fn run(args: &[&str]) -> Output {
    Command::new(callsieve())
        .args(args)
        .output()
        .expect("failed to run callsieve")
}

fn run_with_stdin(args: &[&str], input: &str) -> Output {
    let mut child = Command::new(callsieve())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run callsieve");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("failed to write hook input");
    child
        .wait_with_output()
        .expect("failed to collect callsieve output")
}

fn json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn json_allow_failure(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout should be valid JSON")
}

fn assert_no_codex_unsupported_top_level(output: &Value) {
    assert!(output.get("suppressOutput").is_none());
    assert!(output.get("decision").is_none());
    assert!(output.get("reason").is_none());
}

fn write(path: impl AsRef<Path>, content: &str) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn fixture_repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();

    write(
        root.join(".gitignore"),
        "ignored.ts\nvendor/\nnode_modules/\n",
    );
    write(
        root.join("src/auth/session.ts"),
        "import { tokenFor } from './token';\n\nexport function createSession(userId: string) {\n  return tokenFor(userId);\n}\n\nexport const refreshSession = () => createSession('demo');\n",
    );
    write(
        root.join("src/auth/token.ts"),
        "export function tokenFor(userId: string) {\n  return `token:${userId}`;\n}\n",
    );
    write(
        root.join("src/auth/session.test.ts"),
        "import { createSession } from './session';\n\ntest('createSession returns token-backed session', () => {\n  createSession('demo');\n});\n",
    );
    write(
        root.join("python/services/user_service.py"),
        "import os\nfrom auth.session import create_session\n\nclass UserService:\n    def login_user(self, user_id):\n        return create_session(user_id)\n",
    );
    write(
        root.join("rust/src/lib.rs"),
        "pub struct RequestHandler;\n\nimpl RequestHandler {\n    pub fn handle_request(&self) -> bool {\n        true\n    }\n}\n",
    );
    write(root.join("ignored.ts"), "export function ignored() {}\n");
    write(root.join("vendor/generated.rs"), "pub fn generated() {}\n");

    temp
}

#[test]
fn index_writes_local_json_and_stats_cover_languages() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let index = json(&run(&["index", root]));
    assert_eq!(index["command"], "index");
    assert_eq!(index["files"], 5);
    assert!(index["references"].as_u64().unwrap() > 0);
    assert_eq!(index["lsp_enriched"], false);
    assert!(repo.path().join(".callsieve/index.json").is_file());

    let stats = json(&run(&["stats", root]));
    assert_eq!(stats["files"], 5);
    assert!(stats["references"].as_u64().unwrap() > 0);
    assert_eq!(stats["tests"], 1);
    assert_eq!(stats["languages"]["typescript"], 3);
    assert_eq!(stats["languages"]["python"], 1);
    assert_eq!(stats["languages"]["rust"], 1);
}

#[test]
fn index_includes_docs_configs_and_benchmark_files() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root.join("README.md"),
        "# Agent setup\n\nUse MCP context tools before grep for coding agents.\n",
    );
    write(
        root.join("docs/MCP.md"),
        "# MCP Setup\n\nCall callsieve_context before broad grep or repeated reads.\n",
    );
    write(
        root.join("docs/BENCHMARKS.md"),
        "# Benchmark Evidence\n\nTrace summary and benchmark report workflow for Codex sessions.\n",
    );
    write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\n\n[dependencies]\ntree-sitter-rust = \"0.24\"\n",
    );
    write(
        root.join("benchmarks/tasks.json"),
        r#"{"tasks":[{"task":"benchmark suite evidence"}]}"#,
    );
    write(root.join(".github/workflows/ci.yml"), "name: CI\n");
    write(root.join("Cargo.lock"), "# ignored lockfile\n");
    write(
        root.join("src/lib.rs"),
        "pub fn handler() -> bool { true }\n",
    );
    let root = root.to_str().unwrap();

    let index = json(&run(&["index", root]));
    assert_eq!(index["files"], 7);

    let stats = json(&run(&["stats", root]));
    assert_eq!(stats["languages"]["markdown"], 3);
    assert_eq!(stats["languages"]["toml"], 1);
    assert_eq!(stats["languages"]["json"], 1);
    assert_eq!(stats["languages"]["yaml"], 1);
    assert_eq!(stats["languages"]["rust"], 1);

    let mcp_context = json(&run(&[
        "context",
        root,
        "add MCP context setup docs for agents",
        "--limit",
        "5",
    ]));
    assert!(
        mcp_context["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["file"] == "README.md")
    );
    assert!(
        mcp_context["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["file"] == "docs/MCP.md")
    );

    let docs_context = json(&run(&[
        "context",
        root,
        "update benchmark docs and MCP docs for Codex sessions",
        "--limit",
        "5",
    ]));
    assert!(
        docs_context["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["file"] == "docs/BENCHMARKS.md")
    );
    assert!(
        docs_context["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["file"] == "docs/MCP.md")
    );

    let dependency_context = json(&run(&[
        "context",
        root,
        "change tree-sitter rust dependency config",
        "--limit",
        "5",
    ]));
    let cargo_file = dependency_context["read_first"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["file"] == "Cargo.toml")
        .expect("dependency manifest should be selected");
    assert!(
        cargo_file["why"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "dependency manifest intent")
    );
}

#[test]
fn index_supports_major_language_wave() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root.join("app.php"),
        "<?php\nuse App\\Auth\\Token as Token;\nclass PhpController {}\nfunction php_login() {}\n",
    );
    write(
        root.join("main.go"),
        "package main\nimport \"fmt\"\ntype GoServer struct {}\nfunc HandleGo() {}\n",
    );
    write(
        root.join("User.java"),
        "import com.example.Token;\npublic class JavaService { public void renewSession() {} }\n",
    );
    write(
        root.join("Service.cs"),
        "using System.Text;\npublic class CSharpService { public void Refresh() {} }\n",
    );
    write(root.join("native.h"), "void c_header(void);\n");
    write(
        root.join("native.c"),
        "#include \"native.h\"\nint c_handler() { return 0; }\n",
    );
    write(
        root.join("app.cpp"),
        "#include \"native.h\"\nclass CppService {};\nint cpp_handler() { return 0; }\n",
    );
    write(
        root.join("app.rb"),
        "require \"json\"\nclass RubyService\n  def call\n  end\nend\n",
    );
    write(
        root.join("Main.kt"),
        "import kotlin.collections.List\nclass KotlinService\nfun kotlinLogin() {}\n",
    );
    write(
        root.join("App.swift"),
        "import Foundation\nstruct SwiftService {}\nfunc swiftLogin() {}\n",
    );
    write(
        root.join("App.scala"),
        "import scala.collection.mutable\nclass ScalaService\ndef scalaLogin(): Unit = {}\n",
    );
    write(
        root.join("main.dart"),
        "import 'dart:io';\nclass DartService {}\nvoid dartLogin() {}\n",
    );
    write(
        root.join("mod.lua"),
        "local json = require(\"json\")\nfunction lua_login() end\n",
    );
    write(
        root.join("deploy.sh"),
        "source ./env.sh\nfunction shell_login { echo ok; }\n",
    );
    let root = root.to_str().unwrap();

    let index = json(&run(&["index", root]));
    assert_eq!(index["files"], 14);
    assert!(index["symbols"].as_u64().unwrap() >= 20);
    assert!(index["imports"].as_u64().unwrap() >= 10);

    let stats = json(&run(&["stats", root]));
    for language in [
        "php", "go", "java", "csharp", "c", "cpp", "ruby", "kotlin", "swift", "scala", "dart",
        "lua", "shell",
    ] {
        assert!(
            stats["languages"][language].as_u64().unwrap_or_default() > 0,
            "missing language count for {language}: {}",
            serde_json::to_string_pretty(&stats["languages"]).unwrap()
        );
    }

    let symbols = json(&run(&["symbols", root, "--limit", "100"]));
    let names: Vec<&str> = symbols["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|symbol| symbol["name"].as_str())
        .collect();
    for name in [
        "PhpController",
        "HandleGo",
        "JavaService",
        "CSharpService",
        "c_handler",
        "CppService",
        "RubyService",
        "KotlinService",
        "SwiftService",
        "ScalaService",
        "DartService",
        "lua_login",
        "shell_login",
    ] {
        assert!(names.contains(&name), "missing symbol {name}");
    }

    let context = json(&run(&[
        "agent-context",
        root,
        "change php_login behavior",
        "--limit",
        "5",
    ]));
    assert_eq!(context["context"]["read_first"][0]["file"], "app.php");
}

#[test]
fn symbols_and_symbol_commands_return_indexed_symbols() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let symbols = json(&run(&["symbols", root, "--limit", "50"]));
    let names: Vec<&str> = symbols["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|symbol| symbol["name"].as_str())
        .collect();
    assert!(names.contains(&"createSession"));
    assert!(names.contains(&"UserService"));
    assert!(names.contains(&"handle_request"));

    let symbol = json(&run(&["symbol", root, "createSession", "--limit", "3"]));
    assert_eq!(symbol["matches"][0]["name"], "createSession");
    assert_eq!(symbol["matches"][0]["file"], "src/auth/session.ts");
    assert!(
        symbol["matches"][0]["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|import| import == "src/auth/token.ts")
    );
    assert!(
        symbol["matches"][0]["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call["target"] == "tokenFor" && call["target_file"] == "src/auth/token.ts")
    );
}

#[test]
fn query_ranks_exact_code_context_and_returns_snippet() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let query = json(&run(&[
        "query",
        root,
        "where is createSession auth handled?",
        "--limit",
        "5",
    ]));

    let first = &query["matches"][0];
    assert_eq!(first["file"], "src/auth/session.ts");
    assert_eq!(first["symbol"]["name"], "createSession");
    assert!(first["score"].as_i64().unwrap() > 0);
    assert!(
        first["snippet"]["text"]
            .as_str()
            .unwrap()
            .contains("createSession")
    );
    assert!(!first["why"].as_array().unwrap().is_empty());
    assert!(query["timing"]["ranking_ms"].as_u64().is_some());
    assert!(query["timing"]["index_load_ms"].as_u64().is_some());
}

#[test]
fn query_compacts_related_test_symbols() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root.join("src/api.ts"),
        "export function handleRequest() {\n  return true;\n}\n",
    );
    write(
        root.join("src/api.test.ts"),
        "import { handleRequest } from './api';\n\
         function apiCase01() { return handleRequest(); }\n\
         function apiCase02() { return handleRequest(); }\n\
         function apiCase03() { return handleRequest(); }\n\
         function apiCase04() { return handleRequest(); }\n\
         function apiCase05() { return handleRequest(); }\n\
         function apiCase06() { return handleRequest(); }\n",
    );
    let root = root.to_str().unwrap();
    json(&run(&["index", root]));

    let query = json(&run(&[
        "query",
        root,
        "where is handleRequest implemented",
        "--limit",
        "1",
        "--no-snippets",
    ]));
    let first = &query["matches"][0];
    assert_eq!(first["file"], "src/api.ts");
    let related_tests = first["related_tests"].as_array().unwrap();
    assert_eq!(related_tests.len(), 1);
    assert!(
        related_tests[0]["symbols"].as_array().unwrap().len() <= 5,
        "related test symbols should stay compact: {related_tests:?}"
    );
}

#[test]
fn code_content_terms_help_rank_error_strings() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root.join("src/upload.ts"),
        "export function validateUpload(size: number) {\n  if (size > 10) throw new Error('PAYLOAD_LIMIT_EXCEEDED');\n}\n",
    );
    write(
        root.join("src/noise.ts"),
        "export function unrelated() {\n  return true;\n}\n// PAYLOAD_LIMIT_EXCEEDED PAYLOAD_LIMIT_EXCEEDED\n",
    );
    let root = root.to_str().unwrap();
    json(&run(&["index", root]));

    let query = json(&run(&[
        "query",
        root,
        "where is PAYLOAD_LIMIT_EXCEEDED handled",
        "--limit",
        "3",
        "--no-snippets",
    ]));

    assert_eq!(query["matches"][0]["file"], "src/upload.ts");
}

#[test]
fn hook_meta_tasks_rank_cli_and_tests_first() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    write(
        root.join("src/cli.rs"),
        "fn codex_hooks_doctor() {}\nfn codex_hook_pre_tool_use() {}\nfn hook_doctor() {}\n",
    );
    write(
        root.join("tests/cli.rs"),
        "fn codex_hooks_doctor_smoke_test() {}\n",
    );
    write(
        root.join("src/mcp.rs"),
        "fn tool_execution_error() {}\n// fix fix fix weak weak\n",
    );
    write(
        root.join("benchmarks/proof-sprint-session-trace.example.json"),
        r#"{"proof":"trace proof trace proof","events":[{"command":"proof trace"}]}"#,
    );
    let root = root.to_str().unwrap();
    json(&run(&["index", root]));

    let context = json(&run(&[
        "agent-context",
        root,
        "fix codex hook doctor smoke proof trace",
        "--limit",
        "3",
    ]));
    let files = context["context"]["read_first"].as_array().unwrap();

    assert_eq!(files[0]["file"], "src/cli.rs");
    assert_eq!(files[1]["file"], "tests/cli.rs");
}

#[test]
fn query_and_context_support_why_debug() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let query = json(&run(&[
        "query",
        root,
        "where is createSession auth handled?",
        "--why-debug",
    ]));
    let components = query["matches"][0]["why_debug"].as_array().unwrap();
    assert!(
        components
            .iter()
            .any(|component| component["name"] == "exact_symbol")
    );
    assert!(
        components
            .iter()
            .any(|component| component["points"].as_i64().is_some())
    );

    let context = json(&run(&[
        "context",
        root,
        "change createSession token behavior",
        "--why-debug",
    ]));
    assert!(context["timing"]["graph_expansion_ms"].as_u64().is_some());
    assert!(
        context["read_first"][0]["why_debug"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["name"].as_str().is_some())
    );
}

#[test]
fn context_returns_read_first_packet_for_agent_task() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let context = json(&run(&[
        "context",
        root,
        "change createSession token behavior",
        "--limit",
        "5",
    ]));

    let first = &context["read_first"][0];
    assert_eq!(first["file"], "src/auth/session.ts");
    assert!(!first["symbols"].as_array().unwrap().is_empty());
    assert!(
        first["snippets"][0]["text"]
            .as_str()
            .unwrap()
            .contains("createSession")
    );
    assert!(
        first["related_tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|test| test["file"] == "src/auth/session.test.ts")
    );
    assert!(
        first["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|import| import == "src/auth/token.ts")
    );
    assert!(
        first["referenced_by"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference == "src/auth/session.test.ts")
    );
    assert!(
        first["blast_radius"]["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|import| import == "src/auth/token.ts")
    );
    assert!(
        first["blast_radius"]["tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|test| test == "src/auth/session.test.ts")
    );
    assert!(
        first["blast_radius"]["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/token.ts")
    );
    assert_eq!(first["blast_radius"]["risk"], "medium");
    assert!(
        first["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call["target"] == "tokenFor")
    );
    assert!(!first["why"].as_array().unwrap().is_empty());
}

#[test]
fn context_includes_graph_neighbor_when_limit_allows() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let context = json(&run(&[
        "context",
        root,
        "change createSession behavior",
        "--limit",
        "5",
    ]));

    assert!(
        context["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["file"] == "src/auth/token.ts")
    );
}

#[test]
fn agent_context_wraps_context_with_before_grep_guidance() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let output = json(&run(&[
        "agent-context",
        root,
        "change createSession token behavior",
        "--limit",
        "5",
    ]));

    assert!(
        output["instruction"]["guidance"]
            .as_str()
            .unwrap()
            .contains("grep only if insufficient")
    );
    assert_eq!(
        output["instruction"]["grep_policy"],
        "grep_only_if_context_is_insufficient"
    );
    assert_eq!(
        output["instruction"]["token_policy"],
        "zero_ai_model_tokens_for_retrieval; context_packet_tokens_apply_when_read"
    );
    assert_eq!(
        output["context"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
}

#[test]
fn agent_context_defaults_to_skim_budgeted_packet_without_snippets() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let output = json(&run(&[
        "agent-context",
        root,
        "change createSession token behavior",
    ]));
    let context = &output["context"];
    let first = &context["read_first"][0];

    assert_eq!(context["stats"]["profile"], "skim");
    assert_eq!(context["stats"]["token_budget"], 1200);
    assert_eq!(context["retrieval_cost"]["retrieval_model_tokens"], 0);
    assert_eq!(
        context["retrieval_cost"]["agent_token_cost_scope"],
        "retrieval_only"
    );
    assert!(context["stats"]["estimated_tokens"].as_u64().unwrap() <= 1200);
    assert!(first.get("snippets").is_none());
    assert!(first.get("imports").is_none());
    assert!(first["impact"]["risk"].as_str().is_some());
    assert!(first["symbols"][0].get("signature").is_none());
}

#[test]
fn full_profile_preserves_rich_context_fields() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let output = json(&run(&[
        "agent-context",
        root,
        "change createSession token behavior",
        "--profile",
        "full",
        "--snippets-per-file",
        "1",
        "--token-budget",
        "10000",
    ]));
    let first = &output["context"]["read_first"][0];

    assert_eq!(output["context"]["stats"]["profile"], "full");
    assert!(
        first["snippets"][0]["text"]
            .as_str()
            .unwrap()
            .contains("createSession")
    );
    assert!(
        first["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/token.ts")
    );
    assert!(first["symbols"][0]["signature"].as_str().is_some());
}

#[test]
fn pretty_flag_controls_json_formatting() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let compact = run(&["stats", root]);
    assert!(compact.status.success());
    let compact_stdout = String::from_utf8_lossy(&compact.stdout);
    assert!(!compact_stdout.contains("\n  "));

    let pretty = run(&["--pretty", "stats", root]);
    assert!(pretty.status.success());
    let pretty_stdout = String::from_utf8_lossy(&pretty.stdout);
    assert!(pretty_stdout.contains("\n  "));
}

#[test]
fn context_and_agent_context_support_markdown_output() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let context = run(&[
        "context",
        root,
        "change createSession token behavior",
        "--format",
        "markdown",
    ]);
    assert!(context.status.success());
    let context_stdout = String::from_utf8_lossy(&context.stdout);
    assert!(context_stdout.contains("# CallSieve Context"));
    assert!(context_stdout.contains("Grep policy: grep_only_if_context_is_insufficient"));
    assert!(context_stdout.contains("src/auth/session.ts"));

    let agent = run(&[
        "agent-context",
        root,
        "change createSession token behavior",
        "--format",
        "markdown",
    ]);
    assert!(agent.status.success());
    let agent_stdout = String::from_utf8_lossy(&agent.stdout);
    assert!(agent_stdout.contains("# CallSieve Context"));
    assert!(agent_stdout.contains("function `createSession`"));
}

#[test]
fn focused_followup_commands_return_targeted_detail() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let focus = json(&run(&[
        "focus",
        root,
        "--file",
        "src/auth/session.ts",
        "--symbol",
        "createSession",
    ]));
    assert_eq!(focus["file"], "src/auth/session.ts");
    assert_eq!(focus["symbols"][0]["name"], "createSession");
    assert!(
        focus["snippets"][0]["text"]
            .as_str()
            .unwrap()
            .contains("createSession")
    );

    let related = json(&run(&["related", root, "--file", "src/auth/session.ts"]));
    assert!(
        related["imports"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/token.ts")
    );
    assert_eq!(related["blast_radius"]["risk"], "medium");

    let tests = json(&run(&["tests", root, "--file", "src/auth/session.ts"]));
    assert!(
        tests["related_tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|test| test["file"] == "src/auth/session.test.ts")
    );
}

#[test]
fn mcp_lists_and_calls_context_tool() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let mut child = Command::new(callsieve())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn callsieve mcp");
    let mut stdin = child.stdin.take().unwrap();
    let escaped_root = root.replace('\\', "\\\\");

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"callsieve_context","arguments":{{"path":"{escaped_root}","task":"change createSession token behavior","limit":5}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"callsieve_status","arguments":{{"path":"{escaped_root}"}}}}}}"#
    )
    .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "callsieve");
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "callsieve_context")
    );
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "callsieve_context")
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .contains("Zero-AI-model-token")
    );
    assert_eq!(responses[2]["result"]["isError"], false);
    assert_eq!(
        responses[2]["result"]["structuredContent"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"]["retrieval_cost"]["retrieval_model_tokens"],
        0
    );
    let mcp_text = responses[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(mcp_text.contains("zero AI model tokens"));
    assert!(!mcp_text.contains("\"read_first\""));
    assert_eq!(
        responses[2]["result"]["structuredContent"]["trace_event"]["tool"],
        "callsieve_context"
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"]["freshness"]["initial_fresh"],
        true
    );
    assert_eq!(responses[3]["result"]["isError"], false);
    assert_eq!(
        responses[3]["result"]["structuredContent"]["index_exists"],
        true
    );
}

#[test]
fn mcp_context_rebuilds_missing_and_stale_index() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    let escaped_root = root.replace('\\', "\\\\");

    let mut child = Command::new(callsieve())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn callsieve mcp");
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"callsieve_context","arguments":{{"path":"{escaped_root}","task":"change createSession token behavior","limit":5}}}}}}"#
    )
    .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses[0]["result"]["isError"], false);
    assert_eq!(
        responses[0]["result"]["structuredContent"]["freshness"]["initial_fresh"],
        false
    );
    assert_eq!(
        responses[0]["result"]["structuredContent"]["freshness"]["refreshed"],
        true
    );
    assert!(repo.path().join(".callsieve/index.json").is_file());

    write(
        repo.path().join("src/auth/session.ts"),
        "import { tokenFor } from './token';\n\nexport function createSession(userId: string) {\n  return tokenFor(userId) + ':updated';\n}\n\nexport const refreshSession = () => createSession('demo');\n",
    );

    let mut child = Command::new(callsieve())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn callsieve mcp");
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"callsieve_context","arguments":{{"path":"{escaped_root}","task":"change updated session behavior","limit":5}}}}}}"#
    )
    .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        responses[0]["result"]["structuredContent"]["freshness"]["initial_fresh"],
        false
    );
    assert_eq!(
        responses[0]["result"]["structuredContent"]["freshness"]["final_fresh"],
        true
    );
}

#[test]
fn mcp_context_rebuild_failure_returns_structured_fix() {
    let repo = tempfile::tempdir().unwrap();
    let missing = repo.path().join("missing-repo");
    let escaped_missing = missing.to_string_lossy().replace('\\', "\\\\");

    let mut child = Command::new(callsieve())
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn callsieve mcp");
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"callsieve_context","arguments":{{"path":"{escaped_missing}","task":"change auth","limit":5}}}}}}"#
    )
    .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["structuredContent"]["error"]["fix_command"]
            .as_str()
            .unwrap()
            .contains("callsieve index")
    );
}

#[test]
fn benchmark_returns_grep_vs_context_savings_estimate() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let benchmark = json(&run(&[
        "benchmark",
        root,
        "change createSession token behavior",
        "--limit",
        "5",
    ]));

    assert_eq!(
        benchmark["baseline"]["strategy"],
        "naive grep term scan plus full matched-file reads"
    );
    assert!(
        benchmark["baseline"]["grep_terms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|term| term == "token")
    );
    assert!(
        benchmark["callsieve"]["estimated_packet_tokens"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(
        benchmark["context_payload_reduction"]["retrieval_cost"]["retrieval_model_tokens"],
        0
    );
    assert!(
        benchmark["callsieve"]["top_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["file"] == "src/auth/session.ts")
    );
    assert!(
        benchmark["savings"]["avoided_grep_commands"]
            .as_u64()
            .is_some()
    );
}

#[test]
fn demo_indexes_repo_and_reports_context_reduction() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let demo = json(&run(&[
        "demo",
        root,
        "--task",
        "change createSession token behavior",
    ]));

    assert_eq!(demo["command"], "demo");
    assert_eq!(demo["index"]["files"], 5);
    assert!(
        demo["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );
    assert_eq!(
        demo["context_payload_reduction"]["label"],
        "context_payload_reduction"
    );
    assert_eq!(
        demo["context_payload_reduction"]["retrieval_cost"]["retrieval_model_tokens"],
        0
    );
    assert!(
        demo["next_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command.as_str().unwrap().contains("mcp-config"))
    );
}

#[test]
fn agent_context_reuses_local_task_memory_hints() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let first = json(&run(&[
        "agent-context",
        root,
        "change createSession token behavior",
    ]));
    assert_eq!(first["memory"]["cache_hit"], false);
    assert!(repo.path().join(".callsieve/task-memory.json").is_file());

    let second = json(&run(&[
        "agent-context",
        root,
        "update createSession token behavior",
    ]));
    assert_eq!(second["memory"]["cache_hit"], true);
    assert!(
        second["memory"]["recommended_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );

    let followup = json(&run(&["agent-context", root, "fix 1-5"]));
    assert!(
        followup["context"]["task"]
            .as_str()
            .unwrap()
            .contains("Follow-up: fix 1-5")
    );
    assert!(
        followup["context"]["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["file"] == "src/auth/session.ts")
    );

    let cleared = json(&run(&["memory-clear", root]));
    assert_eq!(cleared["command"], "memory-clear");
    assert_eq!(cleared["removed"], true);
    assert!(!repo.path().join(".callsieve/task-memory.json").is_file());

    let after_clear = json(&run(&[
        "agent-context",
        root,
        "update createSession token behavior",
    ]));
    assert_eq!(after_clear["memory"]["cache_hit"], false);
}

#[test]
fn benchmark_suite_reports_recall_and_observed_session_savings() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let suite_path = repo.path().join("tasks.json");
    write(
        &suite_path,
        r#"{
  "tasks": [
    {
      "id": "auth-session",
      "task": "change createSession token behavior",
      "expected_files": ["src/auth/session.ts", "src/auth/token.ts"],
      "session": {
        "baseline": {
          "grep_commands": 6,
          "file_reads": 9,
          "tokens": 12000,
          "commands": ["rg createSession", "rg token"],
          "files_read": ["src/auth/session.ts", "src/auth/token.ts"]
        },
        "callsieve": {
          "grep_commands": 1,
          "file_reads": 3,
          "tokens": 4000,
          "commands": ["callsieve context"],
          "files_read": ["src/auth/session.ts"]
        }
      }
    }
  ]
}"#,
    );

    let suite = json(&run(&[
        "benchmark-suite",
        root,
        suite_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));

    assert_eq!(suite["task_count"], 1);
    assert_eq!(suite["summary"]["expected_files"], 2);
    assert_eq!(suite["summary"]["expected_files_found"], 2);
    assert_eq!(suite["summary"]["expected_file_recall"], 1.0);
    assert_eq!(suite["summary"]["tasks_with_misses"], 0);
    assert!(
        suite["summary"]["total_estimated_avoided_grep_commands"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(suite["summary"]["observed_session"]["token_savings"], 8000);
    assert_eq!(
        suite["tasks"][0]["observed_session"]["baseline"]["commands"][0],
        "rg createSession"
    );
    assert_eq!(
        suite["tasks"][0]["expected_files_found"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn eval_retrieval_reports_recall_and_fails_on_critical_miss() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let suite_path = repo.path().join("retrieval-eval.json");
    let escaped_root = root.replace('\\', "\\\\");
    write(
        &suite_path,
        &format!(
            r#"{{
  "path": "{escaped_root}",
  "tasks": [
    {{
      "id": "auth-session",
      "task": "change createSession token behavior",
      "expected_files": ["src/auth/session.ts", "src/auth/token.ts"],
      "critical_files": ["src/auth/session.ts"]
    }}
  ]
}}"#
        ),
    );

    let eval = json(&run(&[
        "eval-retrieval",
        suite_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(eval["status"], "pass");
    assert_eq!(eval["summary"]["critical_recall"], 1.0);
    assert!(eval["summary"]["selected_tokens"].as_u64().unwrap() > 0);

    let missed_path = repo.path().join("retrieval-eval-miss.json");
    write(
        &missed_path,
        &format!(
            r#"{{
  "path": "{escaped_root}",
  "tasks": [
    {{
      "id": "missing-critical",
      "task": "change createSession token behavior",
      "expected_files": ["src/auth/session.ts"],
      "critical_files": ["src/auth/missing.ts"]
    }}
  ]
}}"#
        ),
    );

    let missed = run(&[
        "eval-retrieval",
        missed_path.to_str().unwrap(),
        "--limit",
        "5",
    ]);
    assert!(!missed.status.success());
    let missed_json = json_allow_failure(&missed);
    assert_eq!(missed_json["status"], "fail");
    assert_eq!(
        missed_json["tasks"][0]["critical_files_missing"][0],
        "src/auth/missing.ts"
    );
}

#[test]
fn perf_report_emits_latency_percentiles_without_network() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let report = json(&run(&["perf-report", root, "--iterations", "1"]));

    assert_eq!(report["command"], "perf-report");
    assert_eq!(report["iterations"], 1);
    assert!(report["summary"]["p50_ms"].as_u64().is_some());
    assert!(report["summary"]["p95_ms"].as_u64().is_some());
    assert!(report["tasks"].as_array().unwrap().len() > 1);
}

#[test]
fn trace_summary_reports_observed_session_savings() {
    let repo = fixture_repo();
    let trace_path = repo.path().join("trace.json");
    write(
        &trace_path,
        r#"{
  "tasks": [
    {
      "id": "auth-session",
      "task": "change createSession token behavior",
      "expected_files": ["src/auth/session.ts", "src/auth/token.ts"],
      "session": {
        "baseline": {
          "grep_commands": 6,
          "file_reads": 9,
          "tokens": 12000,
          "commands": ["rg createSession", "rg token"],
          "files_read": ["src/auth/session.ts", "src/auth/token.ts"]
        },
        "callsieve": {
          "grep_commands": 1,
          "file_reads": 3,
          "tokens": 4000,
          "commands": ["callsieve context"],
          "files_read": ["src/auth/session.ts"]
        }
      }
    }
  ]
}"#,
    );

    let summary = json(&run(&["trace-summary", trace_path.to_str().unwrap()]));

    assert_eq!(summary["sessions"], 1);
    assert_eq!(summary["baseline_tokens"], 12000);
    assert_eq!(summary["callsieve_tokens"], 4000);
    assert_eq!(summary["token_savings"], 8000);
    assert_eq!(summary["avoided_grep_commands"], 5);
    assert_eq!(summary["avoided_file_reads"], 6);
    assert_eq!(summary["files_still_missed"], 1);
    assert_eq!(summary["missed_files"][0]["files"][0], "src/auth/token.ts");
}

#[test]
fn trace_replay_generates_summary_compatible_trace() {
    let repo = fixture_repo();
    let root = repo.path();
    for index in 0..8 {
        write(
            root.join(format!("src/noise{index}.ts")),
            &format!(
                "export const unrelated{index} = true;\n// {}\n",
                "token ".repeat(1_000)
            ),
        );
    }
    let root_str = root.to_str().unwrap();
    json(&run(&["index", root_str]));

    let suite_path = root.join("tasks.json");
    write(
        &suite_path,
        r#"{"tasks":[{"id":"auth-replay","task":"change createSession token behavior","expected_files":["src/auth/session.ts","src/auth/token.ts"]}]}"#,
    );
    let trace_path = root.join("generated-trace.json");

    let replay = json(&run(&[
        "trace-replay",
        root_str,
        suite_path.to_str().unwrap(),
        trace_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));

    assert!(trace_path.is_file());
    assert_eq!(
        replay["tasks"][0]["session"]["callsieve"]["grep_commands"],
        0
    );
    assert!(
        replay["tasks"][0]["session"]["baseline"]["commands"][0]
            .as_str()
            .unwrap()
            .starts_with("rg -n")
    );
    assert!(
        replay["tasks"][0]["session"]["callsieve"]["commands"][0]
            .as_str()
            .unwrap()
            .contains("callsieve context")
    );
    assert!(
        replay["tasks"][0]["session"]["callsieve"]["notes"][0]
            .as_str()
            .unwrap()
            .contains("Controlled local replay")
    );

    let summary = json(&run(&["trace-summary", trace_path.to_str().unwrap()]));
    assert_eq!(summary["sessions"], 1);
    assert_eq!(summary["files_still_missed"], 0);
    assert!(summary["token_savings"].as_i64().unwrap() > 0);
    assert!(summary["avoided_grep_commands"].as_u64().unwrap() > 0);
    assert!(summary["avoided_file_reads"].as_u64().unwrap() > 0);

    let check = json(&run(&[
        "trace-check",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));
    assert_eq!(check["status"], "pass");
}

#[test]
fn codex_session_writes_model_tagged_trace() {
    let repo = fixture_repo();
    let root = repo.path();
    for index in 0..8 {
        write(
            root.join(format!("src/noise{index}.ts")),
            &format!(
                "export const unrelated{index} = true;\n// {}\n",
                "token ".repeat(1_000)
            ),
        );
    }
    let root_str = root.to_str().unwrap();
    json(&run(&["index", root_str]));
    let trace_path = root.join("codex-session.json");

    let session = json(&run(&[
        "codex-session",
        root_str,
        "change createSession token behavior",
        "--trace-out",
        trace_path.to_str().unwrap(),
        "--model",
        "gpt-5-codex",
        "--expected-file",
        "src/auth/session.ts",
        "--expected-file",
        "src/auth/token.ts",
        "--limit",
        "5",
    ]));

    assert_eq!(session["command"], "codex-session");
    assert_eq!(session["client"], "codex-chatgpt");
    assert_eq!(session["model"], "gpt-5-codex");
    assert_eq!(
        session["context"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
    assert_eq!(session["trace"]["metadata"]["model"], "gpt-5-codex");
    assert!(
        session["trace"]["tasks"][0]["session"]["callsieve"]["commands"][0]
            .as_str()
            .unwrap()
            .contains("callsieve codex-session")
    );
    assert!(trace_path.is_file());

    let summary = json(&run(&["trace-summary", trace_path.to_str().unwrap()]));
    assert_eq!(summary["sessions"], 1);
    assert_eq!(summary["files_still_missed"], 0);
    assert!(summary["token_savings"].as_i64().unwrap() > 0);

    let check = json(&run(&[
        "trace-check",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));
    assert_eq!(check["status"], "pass");
}

#[test]
fn observed_session_lifecycle_writes_summary_compatible_trace() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let trace_path = repo.path().join("observed-session.json");
    let summary_path = repo.path().join("observed-summary.json");

    let start = json(&run(&[
        "session-start",
        root,
        "change createSession token behavior",
        "--client",
        "codex",
        "--model",
        "gpt-5-codex",
        "--trace",
        trace_path.to_str().unwrap(),
        "--expected-file",
        "src/auth/session.ts",
        "--expected-file",
        "src/auth/token.ts",
    ]));
    assert_eq!(start["command"], "session-start");
    assert_eq!(start["collection"], "observed_session");

    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "rg createSession",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "10000",
        "--phase",
        "baseline",
    ]));
    let event = json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "callsieve agent-context . \"change createSession token behavior\"",
        "--context-selected-file",
        "src/auth/session.ts",
        "--context-selected-file",
        "src/auth/token.ts",
        "--tokens",
        "3000",
        "--phase",
        "callsieve",
    ]));
    assert_eq!(event["summary"]["observed_sessions"], 1);
    assert_eq!(event["summary"]["controlled_replay_sessions"], 0);

    let finish = json(&run(&[
        "session-finish",
        trace_path.to_str().unwrap(),
        "--out",
        summary_path.to_str().unwrap(),
    ]));
    assert_eq!(finish["command"], "session-finish");
    assert!(summary_path.is_file());

    let summary = json(&run(&["trace-summary", trace_path.to_str().unwrap()]));
    assert_eq!(summary["sessions"], 1);
    assert_eq!(summary["observed_sessions"], 1);
    assert_eq!(summary["controlled_replay_sessions"], 0);
    assert_eq!(summary["token_savings"], 7000);
    assert_eq!(summary["files_still_missed"], 0);
    assert_eq!(summary["avoided_file_reads"], 1);

    let check = json(&run(&[
        "trace-check",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));
    assert_eq!(check["status"], "pass");
}

#[test]
fn pilot_harness_records_pair_and_finalizes_proof() {
    let repo = fixture_repo();
    for index in 0..8 {
        write(
            repo.path().join(format!("src/noise{index}.ts")),
            &format!(
                "export const unrelated{index} = true;\n// {}\n",
                "token ".repeat(1_000)
            ),
        );
    }
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("pilot.json");

    let init = json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    assert_eq!(init["command"], "pilot-init");
    assert_eq!(init["target_sessions"], 1);
    let initialized_manifest: Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(
        initialized_manifest["protocol"]["evidence_standard"],
        "observed_session_only"
    );
    assert_eq!(
        initialized_manifest["thresholds"]["minimum_planned_tasks"],
        1
    );

    let added = json(&run(&[
        "pilot-task",
        "add",
        manifest_path.to_str().unwrap(),
        root,
        "change createSession token behavior",
        "--id",
        "auth",
        "--expected-file",
        "src/auth/session.ts",
        "--expected-file",
        "src/auth/token.ts",
        "--critical-file",
        "src/auth/session.ts",
        "--critical-file",
        "src/auth/token.ts",
    ]));
    assert_eq!(added["command"], "pilot-task add");
    assert_eq!(added["task"]["status"], "pending");
    assert_eq!(added["task"]["pair_id"], "auth");
    assert_eq!(
        added["task"]["token_accounting_source"],
        "transcript_context_tokens"
    );

    let baseline = json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "rg createSession",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "10000",
    ]));
    assert_eq!(baseline["summary"]["baseline_tokens"], 10000);

    let callsieve = json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "callsieve agent-context . \"change createSession token behavior\"",
        "--files-read",
        "src/auth/session.ts",
        "--files-read",
        "src/auth/token.ts",
        "--tokens",
        "3000",
    ]));
    assert_eq!(callsieve["summary"]["critical_files_still_missed"], 0);

    let qa = json(&run(&["pilot-qa", manifest_path.to_str().unwrap()]));
    assert_eq!(qa["status"], "pass");
    assert_eq!(qa["observed_sessions"], 1);
    assert_eq!(qa["rejected_sessions"], 0);
    assert_eq!(qa["failures"], 0);
    assert!(
        qa["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |check| check["check"] == "countable_observed_session" && check["status"] == "pass"
            )
    );

    let proof_path = manifest_root.path().join("proof.json");
    let finalized = json(&run(&[
        "pilot-finalize",
        manifest_path.to_str().unwrap(),
        "--out",
        proof_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(finalized["command"], "pilot-finalize");
    assert_eq!(
        finalized["proof"]["status"],
        "pass",
        "{}",
        serde_json::to_string_pretty(&finalized).unwrap()
    );
    assert_eq!(finalized["proof"]["proof"]["observed_sessions"], 1);
    assert_eq!(
        finalized["proof"]["proof"]["critical_files_still_missed"],
        0
    );
    assert!(proof_path.is_file());

    let proof_manifest_path = manifest_root.path().join("proof.manifest.json");
    assert!(proof_manifest_path.is_file());
    let proof_manifest: Value =
        serde_json::from_slice(&fs::read(proof_manifest_path).unwrap()).unwrap();
    assert_eq!(proof_manifest["audit"]["planned_tasks"], 1);
    assert_eq!(proof_manifest["audit"]["rejected_sessions"], 0);
    assert_eq!(
        proof_manifest["audit"]["token_accounting_sources"][0],
        "transcript_context_tokens"
    );
    assert!(
        proof_manifest["repos"][0]["policy_trace_paths"][0]
            .as_str()
            .unwrap()
            .contains("callsieve-observed.json")
    );
}

#[test]
fn pilot_qa_allows_uncollected_buffer_tasks_after_target_is_met() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("pilot.json");

    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    for id in ["auth", "buffer"] {
        json(&run(&[
            "pilot-task",
            "add",
            manifest_path.to_str().unwrap(),
            root,
            "change createSession token behavior",
            "--id",
            id,
            "--expected-file",
            "src/auth/session.ts",
            "--critical-file",
            "src/auth/session.ts",
        ]));
    }

    json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "rg createSession",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "10000",
    ]));
    json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "callsieve agent-context . \"change createSession token behavior\"",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "3000",
    ]));

    let qa = json(&run(&["pilot-qa", manifest_path.to_str().unwrap()]));
    assert_eq!(qa["status"], "pass");
    assert_eq!(qa["observed_sessions"], 1);
    assert_eq!(qa["failures"], 0);
    assert!(qa["results"].as_array().unwrap().iter().any(|check| {
        check["task_id"] == "buffer"
            && check["check"] == "combined_trace_exists"
            && check["status"] == "pass"
            && check["message"]
                .as_str()
                .unwrap()
                .contains("uncollected planned buffer task")
    }));

    let proof_path = manifest_root.path().join("proof.json");
    let finalized = json(&run(&[
        "pilot-finalize",
        manifest_path.to_str().unwrap(),
        "--out",
        proof_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(finalized["command"], "pilot-finalize");
    let proof_manifest_path = manifest_root.path().join("proof.manifest.json");
    assert_eq!(
        finalized["proof_manifest"],
        proof_manifest_path.to_string_lossy().into_owned()
    );
    assert!(manifest_root.path().join("proof.manifest.json").is_file());
}

#[test]
fn pilot_init_defaults_to_strict_100_session_claim_protocol() {
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("pilot.json");

    let init = json(&run(&["pilot-init", manifest_path.to_str().unwrap()]));
    assert_eq!(init["target_sessions"], 100);

    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["target_sessions"], 100);
    assert_eq!(manifest["protocol"]["minimum_planned_tasks"], 120);
    assert_eq!(
        manifest["protocol"]["collection"],
        "real_paired_developer_sessions"
    );
    assert_eq!(manifest["thresholds"]["minimum_observed_sessions"], 100);
    assert_eq!(manifest["thresholds"]["minimum_external_repos"], 6);
    assert_eq!(manifest["thresholds"]["minimum_planned_tasks"], 120);
    assert_eq!(
        manifest["thresholds"]["require_transcript_token_accounting"],
        true
    );
    assert_eq!(manifest["thresholds"]["require_codex_bootstrap"], true);
    assert_eq!(manifest["thresholds"]["require_lsp_where_available"], true);
}

#[test]
fn observed_codex_oss_50_milestone_matrix_is_exact() {
    #[derive(Clone)]
    struct ExternalTask {
        repo: &'static str,
        id: String,
        expected_files: Vec<String>,
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let suites = [
        (
            "benchmarks/github-ripgrep",
            "benchmarks/external-ripgrep-suite.json",
        ),
        ("benchmarks/github-fd", "benchmarks/external-fd-suite.json"),
        (
            "benchmarks/github-axum",
            "benchmarks/external-axum-suite.json",
        ),
        (
            "benchmarks/github-flask",
            "benchmarks/external-flask-suite.json",
        ),
        (
            "benchmarks/github-black",
            "benchmarks/external-black-suite.json",
        ),
        (
            "benchmarks/github-httpx",
            "benchmarks/external-httpx-suite.json",
        ),
    ];

    let mut base_tasks = Vec::new();
    for (repo, suite_path) in suites {
        let suite: Value =
            serde_json::from_slice(&fs::read(root.join(suite_path)).unwrap()).unwrap();
        for task in suite["tasks"].as_array().unwrap() {
            assert!(
                task.get("critical_files").is_none(),
                "source suites should leave critical file expansion to the milestone setup"
            );
            let expected_files: Vec<String> = task["expected_files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|file| file.as_str().unwrap().to_string())
                .collect();
            assert!(!expected_files.is_empty());
            base_tasks.push(ExternalTask {
                repo,
                id: task["id"].as_str().unwrap().to_string(),
                expected_files,
            });
        }
    }

    assert_eq!(base_tasks.len(), 12);

    let mut rows = Vec::new();
    for round in 1..=4 {
        for task in &base_tasks {
            rows.push((
                format!("{}-codex-r{round:02}", task.id),
                task.repo,
                task.expected_files.clone(),
                task.expected_files.clone(),
            ));
        }
    }
    for extra_id in ["ripgrep-ignore-walk", "httpx-timeouts-client"] {
        let task = base_tasks
            .iter()
            .find(|task| task.id == extra_id)
            .expect("extra repeat task should exist in base suites");
        rows.push((
            format!("{}-codex-r05", task.id),
            task.repo,
            task.expected_files.clone(),
            task.expected_files.clone(),
        ));
    }

    assert_eq!(rows.len(), 50);
    let ids: BTreeSet<String> = rows.iter().map(|row| row.0.clone()).collect();
    assert_eq!(ids.len(), 50);
    assert!(ids.contains("ripgrep-ignore-walk-codex-r01"));
    assert!(ids.contains("ripgrep-ignore-walk-codex-r05"));
    assert!(ids.contains("httpx-timeouts-client-codex-r05"));

    let repos: BTreeSet<&str> = rows.iter().map(|row| row.1).collect();
    assert_eq!(repos.len(), 6);
    for (_id, _repo, expected_files, critical_files) in rows {
        assert_eq!(critical_files, expected_files);
    }

    let manifest: Value = serde_json::from_slice(
        &fs::read(root.join("benchmarks/evidence/50-session-manifest.example.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["target_sessions"], 50);
    assert_eq!(manifest["thresholds"]["minimum_observed_sessions"], 50);
    assert_eq!(manifest["thresholds"]["minimum_external_repos"], 6);
    assert_eq!(
        manifest["thresholds"]["require_transcript_token_accounting"],
        true
    );
    assert_eq!(
        manifest["thresholds"]["maximum_controlled_replay_ratio"],
        0.0
    );
}

#[test]
fn proof_rehearsal_rust_command_matrix_is_honest() {
    let ledger_root = tempfile::tempdir().unwrap();
    let ledger_path = ledger_root.path().join("rehearsal-run.local.json");
    let output = run(&[
        "proof-rehearsal",
        "--preflight",
        "--ledger",
        ledger_path.to_str().unwrap(),
    ]);
    let rehearsal = json_allow_failure(&output);
    assert_eq!(rehearsal["command"], "proof-rehearsal");
    assert_eq!(rehearsal["mode"], "preflight");
    assert_eq!(rehearsal["command_matrix"]["report_limit"], 24);
    assert_eq!(
        rehearsal["command_matrix"]["context_payload_reduction_included"],
        true
    );
    assert_eq!(
        rehearsal["command_matrix"]["context_payload_scope"],
        "agent_platform_neutral"
    );
    assert_eq!(rehearsal["command_matrix"]["includes_proof_report"], false);
    assert_eq!(rehearsal["claim_proof_included"], false);

    let fixtures = rehearsal["command_matrix"]["external_fixtures"]
        .as_array()
        .unwrap();
    assert_eq!(fixtures.len(), 6);
    let repos: BTreeSet<&str> = fixtures
        .iter()
        .map(|fixture| fixture["repo"].as_str().unwrap())
        .collect();
    let suites: BTreeSet<&str> = fixtures
        .iter()
        .map(|fixture| fixture["suite"].as_str().unwrap())
        .collect();
    let traces: BTreeSet<&str> = fixtures
        .iter()
        .map(|fixture| fixture["trace"].as_str().unwrap())
        .collect();
    let expected_repos = [
        "benchmarks/github-ripgrep",
        "benchmarks/github-fd",
        "benchmarks/github-axum",
        "benchmarks/github-flask",
        "benchmarks/github-black",
        "benchmarks/github-httpx",
    ];
    let expected_suites = [
        "benchmarks/external-ripgrep-suite.json",
        "benchmarks/external-fd-suite.json",
        "benchmarks/external-axum-suite.json",
        "benchmarks/external-flask-suite.json",
        "benchmarks/external-black-suite.json",
        "benchmarks/external-httpx-suite.json",
    ];
    let expected_traces = [
        "benchmarks/external-ripgrep-trace.json",
        "benchmarks/external-fd-trace.json",
        "benchmarks/external-axum-trace.json",
        "benchmarks/external-flask-trace.json",
        "benchmarks/external-black-trace.json",
        "benchmarks/external-httpx-trace.json",
    ];

    for repo in expected_repos {
        assert!(repos.contains(repo), "missing external repo {repo}");
    }
    for suite in expected_suites {
        assert!(suites.contains(suite), "missing external suite {suite}");
    }
    for trace in expected_traces {
        assert!(
            traces.contains(trace),
            "missing controlled replay trace {trace}"
        );
    }

    let output_text = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output_text.contains("proof-report"),
        "rehearsal must not run claim proof"
    );
}

#[test]
fn setup_observed_codex_oss_50_writes_rust_manifest() {
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root
        .path()
        .join("observed-codex-oss-50.local.json");

    let setup = json(&run(&[
        "setup-observed-codex-oss-50",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--skip-repo-check",
    ]));
    assert_eq!(setup["command"], "setup-observed-codex-oss-50");
    assert_eq!(setup["status"], "ready_for_observed_collection");
    assert_eq!(setup["task_count"], 50);
    assert_eq!(setup["target_sessions"], 50);
    assert_eq!(setup["repos"].as_array().unwrap().len(), 6);

    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["target_sessions"], 50);
    assert_eq!(manifest["tasks"].as_array().unwrap().len(), 50);
    assert_eq!(
        manifest["protocol"]["collection"],
        "real_codex_chatgpt_developer_sessions"
    );
    assert_eq!(manifest["thresholds"]["minimum_external_repos"], 6);
    assert_eq!(
        manifest["thresholds"]["require_transcript_token_accounting"],
        true
    );
    assert!(manifest["tasks"].as_array().unwrap().iter().all(|task| {
        task["client"] == "codex"
            && task["model"] == "gpt-5-codex"
            && task["external"] == true
            && task["token_accounting_source"] == "transcript_context_tokens"
            && task["expected_files"] == task["critical_files"]
    }));
}

#[test]
fn setup_observed_claude_oss_50_writes_claude_manifest() {
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root
        .path()
        .join("observed-claude-oss-50.local.json");

    let setup = json(&run(&[
        "setup-observed-claude-oss-50",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--model",
        "claude-opus-4-8",
        "--skip-repo-check",
    ]));
    assert_eq!(setup["command"], "setup-observed-claude-oss-50");
    assert_eq!(setup["status"], "ready_for_observed_collection");
    assert_eq!(setup["task_count"], 50);
    assert_eq!(setup["target_sessions"], 50);

    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(
        manifest["protocol"]["collection"],
        "real_claude_code_developer_sessions"
    );
    assert_eq!(manifest["thresholds"]["require_codex_bootstrap"], false);
    assert!(manifest["tasks"].as_array().unwrap().iter().all(|task| {
        task["client"] == "claude"
            && task["model"] == "claude-opus-4-8"
            && task["external"] == true
            && task["token_accounting_source"] == "transcript_context_tokens"
            && task["id"].as_str().unwrap().contains("-claude-")
            && !task["id"].as_str().unwrap().contains("-codex-")
    }));
}

#[test]
fn codex_observed_recording_rust_helper_validates_inputs_and_wraps_pilot_run() {
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("observed.json");

    let dry_run = json(&run(&[
        "record-codex-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "callsieve agent-context . \"change token expiry\"",
        "--tokens",
        "200",
        "--files-read",
        "src/main.rs",
        "--dry-run",
    ]));
    assert_eq!(dry_run["command"], "record-codex-observed-session");
    assert_eq!(dry_run["status"], "dry_run");
    assert_eq!(dry_run["task_id"], "auth");
    assert_eq!(dry_run["mode"], "callsieve");
    assert_eq!(dry_run["tokens"], 200);
    assert!(dry_run.get("pilot_run").is_none());
    let command = dry_run["pilot_run_command"].as_str().unwrap();
    assert!(command.contains("callsieve pilot-run"));
    assert!(command.contains("--task-id auth"));
    assert!(command.contains("--mode callsieve"));
    assert!(command.contains("--files-read src/main.rs"));
    assert!(command.contains("--tokens 200"));
    assert!(
        dry_run["next_qa"]
            .as_str()
            .unwrap()
            .contains("callsieve pilot-qa")
    );

    let zero_tokens = run(&[
        "record-codex-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "rg auth",
        "--tokens",
        "0",
        "--files-read",
        "src/main.rs",
        "--dry-run",
    ]);
    assert!(!zero_tokens.status.success());
    let zero_tokens = json_allow_failure(&zero_tokens);
    assert!(
        zero_tokens["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Do not estimate tokens")
    );

    let missing_files = run(&[
        "record-codex-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "rg auth",
        "--tokens",
        "100",
        "--dry-run",
    ]);
    assert!(!missing_files.status.success());
    let missing_files = json_allow_failure(&missing_files);
    assert!(
        missing_files["error"]["message"]
            .as_str()
            .unwrap()
            .contains("files_read must include at least one file")
    );
}

#[test]
fn observed_recording_reads_claude_usage_json_and_records_trace_evidence() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("observed-claude.json");
    let usage_path = manifest_root.path().join("claude-auth-baseline.json");
    let stream_usage_path = manifest_root.path().join("claude-auth-callsieve.ndjson");
    write(
        &usage_path,
        r#"{
  "type": "result",
  "result": "done",
  "usage": {
    "input_tokens": 100,
    "cache_creation_input_tokens": 20,
    "cache_read_input_tokens": 30,
    "output_tokens": 10
  }
}
"#,
    );
    let stream_artifact = [
        serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "name": "Read",
                        "input": {
                            "file_path": repo.path().join("src/auth/session.ts").display().to_string()
                        }
                    }
                ]
            }
        })
        .to_string(),
        serde_json::json!({
            "type": "result",
            "usage": {
                "input_tokens": 200,
                "cache_creation_input_tokens": 50,
                "cache_read_input_tokens": 25,
                "output_tokens": 5
            }
        })
        .to_string(),
    ]
    .join("\n");
    write(&stream_usage_path, &stream_artifact);

    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    json(&run(&[
        "pilot-task",
        "add",
        manifest_path.to_str().unwrap(),
        root,
        "change createSession token behavior",
        "--id",
        "auth",
        "--expected-file",
        "src/auth/session.ts",
        "--critical-file",
        "src/auth/session.ts",
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
    ]));

    let recorded = json(&run(&[
        "record-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "claude -p \"change createSession token behavior\" --output-format json",
        "--usage-json",
        usage_path.to_str().unwrap(),
        "--files-read",
        "src/auth/session.ts",
    ]));
    assert_eq!(recorded["command"], "record-observed-session");
    assert_eq!(recorded["status"], "recorded");
    assert_eq!(recorded["client"], "claude");
    assert_eq!(recorded["model"], "claude-opus-4-8");
    assert_eq!(recorded["tokens"], 160);
    assert_eq!(
        recorded["token_input_source"],
        "claude_code_usage_total_tokens"
    );
    assert_eq!(recorded["usage_breakdown"]["total_tokens"], 160);

    let trace_path = manifest_root
        .path()
        .join("tasks")
        .join("auth")
        .join("combined-observed.json");
    let trace: Value = serde_json::from_slice(&fs::read(trace_path).unwrap()).unwrap();
    let event = &trace["events"].as_array().unwrap()[0];
    assert_eq!(
        event["token_evidence"]["accounting_source"],
        "transcript_context_tokens"
    );
    assert_eq!(
        event["token_evidence"]["input_source"],
        "claude_code_usage_total_tokens"
    );
    assert_eq!(
        event["token_evidence"]["claude_code_usage"]["total_tokens"],
        160
    );

    let stream_dry_run = json(&run(&[
        "record-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "claude -p auth --output-format stream-json --verbose",
        "--usage-json",
        stream_usage_path.to_str().unwrap(),
        "--dry-run",
    ]));
    assert_eq!(stream_dry_run["tokens"], 280);
    assert_eq!(stream_dry_run["files_read"][0], "src/auth/session.ts");
    assert_eq!(
        stream_dry_run["usage_breakdown"]["cache_read_input_tokens"],
        25
    );

    let ambiguous = run(&[
        "record-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--client",
        "claude",
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "claude -p auth --output-format json",
        "--tokens",
        "160",
        "--usage-json",
        usage_path.to_str().unwrap(),
        "--files-read",
        "src/auth/session.ts",
        "--dry-run",
    ]);
    assert!(!ambiguous.status.success());
    let ambiguous = json_allow_failure(&ambiguous);
    assert!(
        ambiguous["error"]["message"]
            .as_str()
            .unwrap()
            .contains("either --tokens or --usage-json")
    );
}

#[test]
fn callsieve_observed_recording_counts_context_selected_files_without_reads() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("observed-context-selected.json");
    let baseline_usage_path = manifest_root.path().join("baseline.json");
    let callsieve_usage_path = manifest_root.path().join("callsieve.json");
    write(
        &baseline_usage_path,
        r#"{
  "type": "result",
  "usage": {
    "input_tokens": 120,
    "cache_creation_input_tokens": 20,
    "cache_read_input_tokens": 10,
    "output_tokens": 10
  }
}
"#,
    );
    write(
        &callsieve_usage_path,
        r#"{
  "type": "result",
  "usage": {
    "input_tokens": 25,
    "cache_creation_input_tokens": 5,
    "cache_read_input_tokens": 5,
    "output_tokens": 5
  }
}
"#,
    );

    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    json(&run(&[
        "pilot-task",
        "add",
        manifest_path.to_str().unwrap(),
        root,
        "change createSession token behavior",
        "--id",
        "auth",
        "--expected-file",
        "src/auth/session.ts",
        "--critical-file",
        "src/auth/session.ts",
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
    ]));

    json(&run(&[
        "record-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "claude baseline normal repo search",
        "--usage-json",
        baseline_usage_path.to_str().unwrap(),
        "--files-read",
        "src/auth/session.ts",
    ]));
    let recorded = json(&run(&[
        "record-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "callsieve agent-context . \"change createSession token behavior\" && claude callsieve",
        "--usage-json",
        callsieve_usage_path.to_str().unwrap(),
        "--context-selected-file",
        "src/auth/session.ts",
    ]));

    assert!(recorded["files_read"].as_array().unwrap().is_empty());
    assert_eq!(recorded["context_selected_files"][0], "src/auth/session.ts");
    assert!(
        recorded["pilot_run_command"]
            .as_str()
            .unwrap()
            .contains("--context-selected-file src/auth/session.ts")
    );

    let trace_path = manifest_root
        .path()
        .join("tasks")
        .join("auth")
        .join("combined-observed.json");
    let summary = json(&run(&["trace-summary", trace_path.to_str().unwrap()]));
    assert_eq!(summary["files_still_missed"], 0);
    assert_eq!(summary["critical_files_still_missed"], 0);
    assert_eq!(summary["observed_sessions"], 1);

    let trace: Value = serde_json::from_slice(&fs::read(trace_path).unwrap()).unwrap();
    assert!(
        trace["session"]["callsieve"]["files_read"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        trace["session"]["callsieve"]["context_selected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );

    let qa = json(&run(&["pilot-qa", manifest_path.to_str().unwrap()]));
    assert_eq!(qa["status"], "pass");
    assert_eq!(qa["observed_sessions"], 1);
}

#[test]
fn claude_collector_dry_run_defaults_to_compact_context_snippets() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("observed-claude.json");
    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    json(&run(&[
        "pilot-task",
        "add",
        manifest_path.to_str().unwrap(),
        root,
        "change createSession token behavior",
        "--id",
        "auth",
        "--expected-file",
        "src/auth/session.ts",
        "--critical-file",
        "src/auth/session.ts",
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
    ]));

    let dry_run = json(&run(&[
        "collect-claude-observed-session",
        "--manifest",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--model",
        "claude-opus-4-8",
        "--dry-run",
    ]));

    assert_eq!(dry_run["status"], "dry_run");
    assert_eq!(dry_run["snippets_per_file"], 1);
    assert!(dry_run["prompt_tokens_estimate"].as_u64().unwrap() > 0);
    assert!(dry_run.get("record").is_none());
    assert!(
        dry_run["context_selected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );
}

#[test]
fn proof_sprint_init_status_and_dry_run_collect_wrap_existing_claude_flow() {
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("proof-sprint.local.json");

    let init = json(&run(&[
        "proof-sprint",
        "init",
        manifest_path.to_str().unwrap(),
        "--client",
        "claude",
        "--sessions",
        "10",
        "--model",
        "claude-opus-4-8",
        "--skip-repo-check",
    ]));
    assert_eq!(init["command"], "proof-sprint init");
    assert_eq!(init["status"], "ready_for_observed_collection");
    assert_eq!(init["client"], "claude");
    assert_eq!(init["task_count"], 50);
    assert_eq!(init["target_sessions"], 10);
    assert!(
        init["next_collect"]
            .as_str()
            .unwrap()
            .contains("--mode baseline")
    );

    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["target_sessions"], 10);
    assert_eq!(manifest["thresholds"]["minimum_observed_sessions"], 10);
    assert_eq!(manifest["tasks"].as_array().unwrap().len(), 50);
    assert!(manifest["tasks"].as_array().unwrap().iter().all(|task| {
        task["client"] == "claude"
            && task["model"] == "claude-opus-4-8"
            && task["token_accounting_source"] == "transcript_context_tokens"
    }));
    let first_task = manifest["tasks"][0]["id"].as_str().unwrap();

    let status = json(&run(&[
        "proof-sprint",
        "status",
        manifest_path.to_str().unwrap(),
    ]));
    assert_eq!(status["command"], "proof-sprint status");
    assert_eq!(status["status"], "collecting");
    assert_eq!(status["paired_sessions_complete"], 0);
    assert_eq!(
        status["missing_baseline_phases"].as_array().unwrap().len(),
        50
    );
    assert_eq!(
        status["transcript_accounting_coverage_percent"]
            .as_f64()
            .unwrap(),
        0.0
    );
    assert!(
        status["next_command"]
            .as_str()
            .unwrap()
            .contains(&format!("--task-id {first_task} --mode baseline"))
    );

    let dry_run = json(&run(&[
        "proof-sprint",
        "collect",
        manifest_path.to_str().unwrap(),
        "--task-id",
        first_task,
        "--mode",
        "baseline",
        "--dry-run",
    ]));
    assert_eq!(dry_run["command"], "proof-sprint collect");
    assert_eq!(dry_run["status"], "dry_run");
    assert_eq!(dry_run["model"], "claude-opus-4-8");
    assert_eq!(
        dry_run["collect"]["command"],
        "collect-claude-observed-session"
    );
    assert!(
        dry_run["collect"]["claude_command"]
            .as_str()
            .unwrap()
            .contains("claude -p")
    );
    assert!(dry_run["collect"].get("record").is_none());

    let run_dry = json(&run(&[
        "proof-sprint",
        "run",
        manifest_path.to_str().unwrap(),
        "--dry-run",
    ]));
    assert_eq!(run_dry["command"], "proof-sprint run");
    assert_eq!(run_dry["status"], "dry_run");
    assert_eq!(run_dry["collected_phases"], 0);
    assert_eq!(run_dry["phases"].as_array().unwrap().len(), 1);
    assert_eq!(run_dry["phases"][0]["task_id"], first_task);
    assert_eq!(run_dry["phases"][0]["mode"], "baseline");
    assert_eq!(run_dry["phases"][0]["collect"]["status"], "dry_run");
}

#[test]
fn proof_sprint_status_pairs_baseline_then_callsieve_before_new_task() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("proof-sprint.json");

    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    json(&run(&[
        "pilot-task",
        "add",
        manifest_path.to_str().unwrap(),
        root,
        "change createSession token behavior",
        "--id",
        "auth",
        "--expected-file",
        "src/auth/session.ts",
        "--critical-file",
        "src/auth/session.ts",
        "--client",
        "claude",
        "--model",
        "claude-opus-4-8",
    ]));

    let empty = json(&run(&[
        "proof-sprint",
        "status",
        manifest_path.to_str().unwrap(),
    ]));
    assert!(
        empty["next_command"]
            .as_str()
            .unwrap()
            .contains("--task-id auth --mode baseline")
    );

    json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "rg createSession",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "10000",
    ]));
    let run_without_resume = run(&[
        "proof-sprint",
        "run",
        manifest_path.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(!run_without_resume.status.success());
    let run_without_resume = json_allow_failure(&run_without_resume);
    assert!(
        run_without_resume["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pass --resume")
    );

    let baseline_only = json(&run(&[
        "proof-sprint",
        "status",
        manifest_path.to_str().unwrap(),
    ]));
    assert!(
        baseline_only["next_command"]
            .as_str()
            .unwrap()
            .contains("--task-id auth --mode callsieve")
    );
    let resumed_run = json(&run(&[
        "proof-sprint",
        "run",
        manifest_path.to_str().unwrap(),
        "--resume",
        "--dry-run",
    ]));
    assert_eq!(resumed_run["status"], "dry_run");
    assert_eq!(resumed_run["phases"][0]["task_id"], "auth");
    assert_eq!(resumed_run["phases"][0]["mode"], "callsieve");

    json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "callsieve agent-context . \"change createSession token behavior\"",
        "--context-selected-file",
        "src/auth/session.ts",
        "--tokens",
        "3000",
    ]));
    let complete = json(&run(&[
        "proof-sprint",
        "status",
        manifest_path.to_str().unwrap(),
    ]));
    assert_eq!(complete["status"], "ready_to_finalize");
    assert_eq!(complete["paired_sessions_complete"], 1);
    assert_eq!(complete["qa_status"], "pass");
    assert!(
        complete["next_command"]
            .as_str()
            .unwrap()
            .contains("proof-sprint finalize")
    );
    assert!(
        complete["observed_token_reduction_percent"]
            .as_f64()
            .unwrap()
            > 60.0
    );
}

#[test]
fn proof_sprint_finalize_refuses_until_pilot_qa_passes() {
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("proof-sprint.json");
    let proof_path = manifest_root.path().join("proof.json");
    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));

    let output = run(&[
        "proof-sprint",
        "finalize",
        manifest_path.to_str().unwrap(),
        "--out",
        proof_path.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    let error = json_allow_failure(&output);
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("pilot QA failed")
    );
    assert!(!proof_path.exists());
}

#[test]
fn pilot_task_reject_preserves_audit_and_excludes_task_from_count() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("pilot.json");

    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    json(&run(&[
        "pilot-task",
        "add",
        manifest_path.to_str().unwrap(),
        root,
        "change createSession token behavior",
        "--id",
        "auth",
        "--expected-file",
        "src/auth/session.ts",
        "--critical-file",
        "src/auth/session.ts",
    ]));

    let rejected = json(&run(&[
        "pilot-task",
        "reject",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--reason",
        "operator saw answer during baseline phase",
    ]));
    assert_eq!(rejected["command"], "pilot-task reject");
    assert_eq!(rejected["rejected"]["task_id"], "auth");

    let qa = json(&run(&["pilot-qa", manifest_path.to_str().unwrap()]));
    assert_eq!(qa["status"], "fail");
    assert_eq!(qa["observed_sessions"], 0);
    assert_eq!(qa["rejected_sessions"], 1);
    assert!(
        qa["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "rejected_session_audit" && check["status"] == "pass")
    );

    let output = run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "callsieve agent-context . \"change createSession token behavior\"",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "3000",
    ]);
    assert!(!output.status.success());
}

#[test]
fn pilot_qa_fails_when_callsieve_misses_critical_file() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("pilot.json");

    json(&run(&[
        "pilot-init",
        manifest_path.to_str().unwrap(),
        "--sessions",
        "1",
    ]));
    json(&run(&[
        "pilot-task",
        "add",
        manifest_path.to_str().unwrap(),
        root,
        "change createSession token behavior",
        "--id",
        "auth",
        "--expected-file",
        "src/auth/session.ts",
        "--critical-file",
        "src/auth/token.ts",
    ]));
    json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "baseline",
        "--command",
        "rg createSession",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "10000",
    ]));
    json(&run(&[
        "pilot-run",
        manifest_path.to_str().unwrap(),
        "--task-id",
        "auth",
        "--mode",
        "callsieve",
        "--command",
        "callsieve agent-context . \"change createSession token behavior\"",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "3000",
    ]));

    let qa = json(&run(&["pilot-qa", manifest_path.to_str().unwrap()]));
    assert_eq!(qa["status"], "fail");
    assert!(
        qa["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "critical_misses" && check["status"] == "fail")
    );
}

#[test]
fn proof_report_requires_observed_sessions_and_rejects_mislabeled_replay() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let suite = repo.path().join("tasks.json");
    write(
        &suite,
        r#"{"tasks":[{"id":"auth","task":"change createSession token behavior","expected_files":["src/auth/session.ts","src/auth/token.ts"]}]}"#,
    );
    let trace = repo.path().join("observed.json");
    write(
        &trace,
        r#"{
  "metadata": {"collection": "observed_session", "client": "codex", "model": "gpt-5-codex"},
  "task": "change createSession token behavior",
  "expected_files": ["src/auth/session.ts"],
  "baseline": {
    "grep_commands": 4,
    "file_reads": 5,
    "tokens": 10000,
    "commands": ["rg createSession"],
    "files_read": ["src/auth/session.ts"]
  },
  "callsieve": {
    "grep_commands": 0,
    "file_reads": 2,
    "tokens": 3000,
    "commands": ["callsieve agent-context . \"change auth\""],
    "files_read": ["src/auth/session.ts"]
  }
}"#,
    );
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("proof.json");
    let escaped_root = root.replace('\\', "\\\\");
    let escaped_suite = suite.to_string_lossy().replace('\\', "\\\\");
    let escaped_trace = trace.to_string_lossy().replace('\\', "\\\\");
    write(
        &manifest_path,
        &format!(
            r#"{{
  "thresholds": {{
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": -300.0,
    "minimum_observed_sessions": 1,
    "minimum_observed_token_reduction_percent": 50.0,
    "maximum_controlled_replay_ratio": 0.0,
    "maximum_trace_violations": 0,
    "require_fresh_index": true
  }},
  "repos": [
    {{"label":"fixture","path":"{escaped_root}","suite_path":"{escaped_suite}","trace_path":"{escaped_trace}"}}
  ]
}}"#
        ),
    );

    let proof = json(&run(&[
        "proof-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(proof["command"], "proof-report");
    assert_eq!(proof["status"], "pass");
    assert_eq!(proof["proof"]["observed_sessions"], 1);
    assert_eq!(proof["proof"]["controlled_replay_sessions"], 0);

    write(
        &trace,
        r#"{
  "metadata": {"collection": "observed_session", "client": "codex", "model": "gpt-5-codex"},
  "tasks": [
    {
      "id": "bad",
      "task": "change auth",
      "session": {
        "baseline": {
          "grep_commands": 2,
          "file_reads": 3,
          "tokens": 1000,
          "commands": ["rg auth"],
          "files_read": ["src/auth/session.ts"],
          "notes": ["Controlled local replay, not human-session telemetry."]
        },
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 1,
          "tokens": 500,
          "commands": ["callsieve codex-session . \"change auth\""],
          "files_read": ["src/auth/session.ts"]
        }
      }
    }
  ]
}"#,
    );
    let failed = json(&run(&[
        "proof-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(failed["status"], "fail");
    assert!(
        failed["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "observed_trace_mislabeled_controlled_replay")
    );

    write(
        &trace,
        r#"{
  "metadata": {"collection": "codex_hook_trace", "client": "codex", "model": "gpt-5-codex"},
  "task": "change createSession token behavior",
  "expected_files": ["src/auth/session.ts"],
  "baseline": {
    "grep_commands": 4,
    "file_reads": 5,
    "tokens": 10000,
    "commands": ["rg createSession"],
    "files_read": ["src/auth/session.ts"]
  },
  "callsieve": {
    "grep_commands": 0,
    "file_reads": 0,
    "tokens": 3000,
    "commands": ["callsieve agent-context . \"change auth\""],
    "files_read": [],
    "context_selected_files": ["src/auth/session.ts"]
  },
  "policy": {"source": "codex_lifecycle_hooks"},
  "events": [{"hook_event": "UserPromptSubmit"}]
}"#,
    );
    let hook_trace = json(&run(&[
        "proof-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(hook_trace["status"], "fail");
    assert_eq!(hook_trace["proof"]["observed_sessions"], 0);
    assert!(
        hook_trace["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "minimum_observed_sessions")
    );

    write(
        &trace,
        r#"{
  "metadata": {"collection": "observed_session", "client": "codex", "model": "gpt-5-codex"},
  "task": "change createSession token behavior",
  "expected_files": ["src/auth/session.ts"],
  "baseline": {
    "grep_commands": 4,
    "file_reads": 5,
    "tokens": 10000,
    "commands": ["rg createSession"],
    "files_read": ["src/auth/session.ts"]
  },
  "callsieve": {
    "grep_commands": 0,
    "file_reads": 0,
    "tokens": 3000,
    "commands": ["callsieve agent-context . \"change auth\""],
    "files_read": [],
    "context_selected_files": ["src/auth/session.ts"]
  },
  "policy": {"source": "codex_lifecycle_hooks"},
  "events": [{"hook_event": "UserPromptSubmit"}]
}"#,
    );
    let mislabeled_hook = json(&run(&[
        "proof-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(mislabeled_hook["status"], "fail");
    assert!(
        mislabeled_hook["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "observed_trace_mislabeled_hook_trace")
    );
}

#[test]
fn proof_report_requires_transcript_token_provenance_when_strict() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let suite = repo.path().join("tasks.json");
    write(
        &suite,
        r#"{"tasks":[{"id":"auth","task":"change createSession token behavior","expected_files":["src/auth/session.ts"]}]}"#,
    );
    let trace = repo.path().join("observed.json");
    write(
        &trace,
        r#"{
  "metadata": {"collection": "observed_session", "client": "codex", "model": "gpt-5-codex"},
  "task": "change createSession token behavior",
  "expected_files": ["src/auth/session.ts"],
  "baseline": {
    "grep_commands": 4,
    "file_reads": 5,
    "tokens": 10000,
    "commands": ["rg createSession"],
    "files_read": ["src/auth/session.ts"]
  },
  "callsieve": {
    "grep_commands": 0,
    "file_reads": 2,
    "tokens": 3000,
    "commands": ["callsieve agent-context . \"change auth\""],
    "files_read": ["src/auth/session.ts"]
  }
}"#,
    );
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("proof.json");
    let escaped_root = root.replace('\\', "\\\\");
    let escaped_suite = suite.to_string_lossy().replace('\\', "\\\\");
    let escaped_trace = trace.to_string_lossy().replace('\\', "\\\\");
    write(
        &manifest_path,
        &format!(
            r#"{{
  "thresholds": {{
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": -300.0,
    "minimum_observed_sessions": 1,
    "minimum_observed_token_reduction_percent": 50.0,
    "minimum_planned_tasks": 1,
    "maximum_controlled_replay_ratio": 0.0,
    "maximum_trace_violations": 0,
    "require_fresh_index": true,
    "require_transcript_token_accounting": true
  }},
  "audit": {{
    "planned_tasks": 1,
    "rejected_sessions": 0,
    "token_accounting_sources": ["manual_entry"]
  }},
  "repos": [
    {{"label":"fixture","path":"{escaped_root}","suite_path":"{escaped_suite}","trace_path":"{escaped_trace}"}}
  ]
}}"#
        ),
    );

    let proof = json(&run(&[
        "proof-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));
    assert_eq!(proof["status"], "fail");
    assert!(
        proof["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "require_transcript_token_accounting")
    );
}

#[test]
fn enterprise_proof_report_fails_below_1000_observed_sessions() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let suite = repo.path().join("tasks.json");
    write(
        &suite,
        r#"{"tasks":[{"id":"auth","task":"change createSession token behavior","expected_files":["src/auth/session.ts"],"task_category":"bug_fix"}]}"#,
    );
    let trace = repo.path().join("observed.json");
    write(
        &trace,
        r#"{
  "metadata": {"collection": "observed_session", "client": "codex", "model": "gpt-5-codex"},
  "token_accounting": {"source": "transcript_context_tokens"},
  "task": "change createSession token behavior",
  "task_category": "bug_fix",
  "expected_files": ["src/auth/session.ts"],
  "baseline": {
    "grep_commands": 4,
    "file_reads": 5,
    "tokens": 10000,
    "commands": ["rg createSession"],
    "files_read": ["src/auth/session.ts"]
  },
  "callsieve": {
    "grep_commands": 0,
    "file_reads": 2,
    "tokens": 3000,
    "commands": ["callsieve agent-context . \"change auth\""],
    "files_read": ["src/auth/session.ts"]
  }
}"#,
    );
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("enterprise.json");
    let escaped_root = root.replace('\\', "\\\\");
    let escaped_suite = suite.to_string_lossy().replace('\\', "\\\\");
    let escaped_trace = trace.to_string_lossy().replace('\\', "\\\\");
    write(
        &manifest_path,
        &format!(
            r#"{{
  "protocol": "enterprise-proof",
  "thresholds": {{
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": -300.0,
    "minimum_observed_sessions": 1000,
    "minimum_observed_token_reduction_percent": 50.0,
    "maximum_controlled_replay_ratio": 0.0,
    "maximum_trace_violations": 0,
    "maximum_critical_misses": 0,
    "require_transcript_token_accounting": true
  }},
  "repos": [
    {{
      "label": "fixture",
      "path": "{escaped_root}",
      "suite_path": "{escaped_suite}",
      "trace_path": "{escaped_trace}",
      "clients": ["codex"],
      "task_categories": ["bug_fix"]
    }}
  ]
}}"#
        ),
    );

    let proof = json(&run(&[
        "enterprise-proof-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));

    assert_eq!(proof["command"], "enterprise-proof-report");
    assert_eq!(proof["status"], "fail");
    assert_eq!(proof["proof"]["observed_sessions"], 1);
    assert!(
        proof["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "minimum_observed_sessions")
    );
}

#[test]
fn enterprise_proof_report_requires_clients_and_session_savings_ratios() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let suite = repo.path().join("tasks.json");
    write(
        &suite,
        r#"{"tasks":[
  {"id":"bug","task":"change createSession token behavior","expected_files":["src/auth/session.ts"],"task_category":"bug_fix"},
  {"id":"docs","task":"update auth documentation","expected_files":["src/auth/session.ts"],"task_category":"docs"}
]}"#,
    );
    let trace = repo.path().join("observed.json");
    write(
        &trace,
        r#"{
  "metadata": {"collection": "observed_session", "client": "codex", "model": "gpt-5-codex"},
  "token_accounting": {"source": "transcript_context_tokens"},
  "tasks": [
    {
      "id": "bug",
      "task": "change createSession token behavior",
      "task_category": "bug_fix",
      "expected_files": ["src/auth/session.ts"],
      "session": {
        "baseline": {
          "grep_commands": 4,
          "file_reads": 5,
          "tokens": 10000,
          "commands": ["rg createSession"],
          "files_read": ["src/auth/session.ts"]
        },
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 2,
          "tokens": 5000,
          "commands": ["callsieve agent-context . \"change auth\""],
          "files_read": ["src/auth/session.ts"]
        }
      }
    },
    {
      "id": "docs",
      "task": "update auth documentation",
      "task_category": "docs",
      "expected_files": ["src/auth/session.ts"],
      "session": {
        "baseline": {
          "grep_commands": 2,
          "file_reads": 3,
          "tokens": 10000,
          "commands": ["rg auth"],
          "files_read": ["src/auth/session.ts"]
        },
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 2,
          "tokens": 12000,
          "commands": ["callsieve agent-context . \"update auth docs\""],
          "files_read": ["src/auth/session.ts"]
        }
      }
    }
  ]
}"#,
    );
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("enterprise.json");
    let escaped_root = root.replace('\\', "\\\\");
    let escaped_suite = suite.to_string_lossy().replace('\\', "\\\\");
    let escaped_trace = trace.to_string_lossy().replace('\\', "\\\\");
    write(
        &manifest_path,
        &format!(
            r#"{{
  "protocol": "enterprise-proof",
  "thresholds": {{
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": -300.0,
    "minimum_observed_sessions": 2,
    "minimum_clients": 3,
    "required_clients": ["codex", "claude", "cursor"],
    "minimum_positive_savings_session_percent": 90.0,
    "minimum_sessions_over_30_percent_savings_percent": 75.0,
    "maximum_controlled_replay_ratio": 0.0,
    "maximum_trace_violations": 0,
    "maximum_critical_misses": 0,
    "require_transcript_token_accounting": true
  }},
  "repos": [
    {{
      "label": "fixture",
      "path": "{escaped_root}",
      "suite_path": "{escaped_suite}",
      "trace_path": "{escaped_trace}",
      "clients": ["codex"],
      "task_categories": ["bug_fix", "docs"]
    }}
  ]
}}"#
        ),
    );

    let proof = json(&run(&[
        "enterprise-proof-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));

    assert_eq!(proof["status"], "fail");
    assert_eq!(proof["proof"]["observed_sessions"], 2);
    assert_eq!(proof["proof"]["positive_savings_sessions"], 1);
    assert_eq!(proof["proof"]["sessions_over_30_percent_savings"], 1);
    assert!(
        proof["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "minimum_clients")
    );
    assert!(
        proof["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "required_clients")
    );
    assert!(
        proof["failures"]
            .as_array()
            .unwrap()
            .iter()
            .any(|failure| failure["check"] == "minimum_positive_savings_session_percent")
    );
    assert!(
        proof["failures"].as_array().unwrap().iter().any(|failure| {
            failure["check"] == "minimum_sessions_over_30_percent_savings_percent"
        })
    );
}

#[test]
fn evidence_pack_preserves_pmf_metrics_and_redacts_team_identifiers() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let suite = repo.path().join("tasks.json");
    write(
        &suite,
        r#"{"tasks":[{"id":"auth","task":"change createSession token behavior","expected_files":["src/auth/session.ts"],"task_category":"bug_fix"}]}"#,
    );
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("enterprise.json");
    let escaped_root = root.replace('\\', "\\\\");
    let escaped_suite = suite.to_string_lossy().replace('\\', "\\\\");
    write(
        &manifest_path,
        &format!(
            r#"{{
  "protocol": "enterprise-proof",
  "thresholds": {{
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": -300.0,
    "minimum_pilot_teams": 5,
    "minimum_paid_or_converted_teams": 3,
    "minimum_teams_with_20_sessions": 4,
    "minimum_meaningfully_worse_without_teams": 3,
    "minimum_case_study_teams": 2,
    "minimum_renewal_or_loi_teams": 1
  }},
  "audit": {{
    "product_market": {{
      "teams_completed_pilots": 5,
      "paid_pilot_or_converted_teams": 3,
      "teams_with_20_plus_sessions": 4,
      "meaningfully_worse_without_teams": 3,
      "quote_approved_case_study_teams": 2,
      "renewal_expansion_or_loi_teams": 1
    }}
  }},
  "repos": [
    {{
      "label": "enterprise-alpha",
      "team": "Payments Platform",
      "path": "{escaped_root}",
      "suite_path": "{escaped_suite}",
      "clients": ["codex"],
      "task_categories": ["bug_fix"],
      "scale_class": "paid_pilot"
    }}
  ]
}}"#
        ),
    );

    let evidence = json(&run(&[
        "evidence-pack",
        manifest_path.to_str().unwrap(),
        "--anonymize",
        "--limit",
        "5",
    ]));

    assert_eq!(evidence["command"], "evidence-pack");
    assert_eq!(evidence["anonymized"], true);
    assert!(
        evidence["protocol"]
            .as_str()
            .unwrap()
            .contains("enterprise-proof")
    );
    assert_eq!(evidence["evidence"]["status"], "pass");
    assert_eq!(
        evidence["evidence"]["proof"]["product_market"]["teams_completed_pilots"],
        5
    );
    assert_eq!(evidence["evidence"]["repos"][0]["label"], "<redacted>");
    assert_eq!(evidence["evidence"]["repos"][0]["team"], "<redacted>");
    assert_eq!(
        evidence["evidence"]["benchmark"]["repos"][0]["team"],
        "<redacted>"
    );
}

#[test]
fn codex_bootstrap_generates_local_files_and_enforce_passes_with_path_shim() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let bootstrap = json(&run(&[
        "codex-bootstrap",
        root,
        "--model",
        "gpt-5-codex",
        "--force",
    ]));
    assert_eq!(bootstrap["command"], "codex-bootstrap");
    assert!(repo.path().join(".codex/config.toml").is_file());
    assert!(repo.path().join(".codex/CALLSIEVE.md").is_file());
    assert!(repo.path().join(".callsieve/codex-launch.ps1").is_file());
    assert!(repo.path().join(".callsieve/codex-launch.sh").is_file());
    let config = fs::read_to_string(repo.path().join(".codex/config.toml")).unwrap();
    assert!(config.contains("[mcp_servers.callsieve]"));
    assert!(!config.contains("command = \"callsieve\""));
    assert!(config.contains("callsieve"));
    let callsieve_launcher = if cfg!(windows) {
        repo.path().join(".callsieve/bin/callsieve.cmd")
    } else {
        repo.path().join(".callsieve/bin/callsieve")
    };
    assert!(callsieve_launcher.is_file());
    let launcher_output = Command::new(callsieve_launcher)
        .arg("--version")
        .output()
        .expect("failed to run project-local callsieve launcher");
    assert!(
        launcher_output.status.success(),
        "launcher failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&launcher_output.stdout),
        String::from_utf8_lossy(&launcher_output.stderr)
    );

    let trace = repo.path().join("observed.json");
    write(
        &trace,
        r#"{
  "metadata": {"collection": "observed_session", "client": "codex", "model": "gpt-5-codex"},
  "baseline": {"grep_commands": 1, "file_reads": 1, "tokens": 1000, "commands": ["rg auth"]},
  "callsieve": {"grep_commands": 0, "file_reads": 1, "tokens": 500, "commands": ["callsieve agent-context . \"change auth\""]}
}"#,
    );
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let shim_dir = repo.path().join(".callsieve/bin");
    let mut paths = vec![shim_dir];
    paths.extend(std::env::split_paths(&old_path));
    let joined = std::env::join_paths(paths).unwrap();
    let output = Command::new(callsieve())
        .args([
            "enforce",
            root,
            "--client",
            "codex",
            "--trace",
            trace.to_str().unwrap(),
            "--strict",
            "--require-shim",
        ])
        .env("PATH", joined)
        .output()
        .expect("failed to run callsieve");
    let enforce = json(&output);
    assert_eq!(
        enforce["status"],
        "pass",
        "{}",
        serde_json::to_string_pretty(&enforce).unwrap()
    );
}

#[test]
fn bootstrap_generic_strict_builds_local_adoption_stack() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let output = Command::new(callsieve())
        .args([
            "bootstrap",
            root,
            "--client",
            "generic",
            "--strict",
            "--force",
        ])
        .env("CALLSIEVE_TEST_BACKGROUND_NO_SPAWN", "1")
        .output()
        .expect("failed to run callsieve");
    let bootstrap = json(&output);

    assert_eq!(
        bootstrap["status"],
        "pass",
        "{}",
        serde_json::to_string_pretty(&bootstrap).unwrap()
    );
    assert_eq!(bootstrap["command"], "bootstrap");
    assert!(repo.path().join(".callsieve/index.json").is_file());
    assert!(repo.path().join(".callsieve/agent-policy.md").is_file());
    assert!(repo.path().join(".callsieve/bin").is_dir());
    assert!(repo.path().join(".callsieve/daemon.json").is_file());
    assert_eq!(bootstrap["daemon"]["mode"], "background");
    assert_eq!(bootstrap["daemon"]["pid"], 0);
    assert_eq!(bootstrap["enforcement"]["status"], "pass");
    assert!(
        bootstrap["first_required_command"]
            .as_str()
            .unwrap()
            .contains("callsieve agent-context")
    );
}

#[test]
fn doctor_reports_missing_setup_and_fix_repairs_it() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let report = json(&run(&["doctor", root, "--client", "generic", "--strict"]));
    assert_eq!(report["status"], "fail");
    assert!(!repo.path().join(".callsieve/index.json").is_file());
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "fresh_index" && check["status"] == "fail")
    );

    let output = Command::new(callsieve())
        .args(["doctor", root, "--client", "generic", "--fix", "--strict"])
        .env("CALLSIEVE_TEST_BACKGROUND_NO_SPAWN", "1")
        .output()
        .expect("failed to run callsieve");
    let fixed = json(&output);
    assert_eq!(
        fixed["status"],
        "pass",
        "{}",
        serde_json::to_string_pretty(&fixed).unwrap()
    );
    assert!(repo.path().join(".callsieve/index.json").is_file());
    assert!(repo.path().join(".callsieve/agent-policy.md").is_file());
    assert!(repo.path().join(".callsieve/bin").is_dir());
    assert!(repo.path().join(".callsieve/daemon.json").is_file());
    assert!(
        fixed["fixes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|fix| fix["step"] == "strict_shim")
    );
}

#[test]
fn editor_hook_generates_project_local_files_only() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();

    let cursor = json(&run(&[
        "editor-hook",
        root,
        "--editor",
        "cursor",
        "--force",
    ]));
    assert_eq!(cursor["command"], "editor-hook");
    assert_eq!(cursor["editor"], "cursor");
    assert!(repo.path().join(".cursor/mcp.json").is_file());
    assert!(repo.path().join(".cursor/rules/callsieve.mdc").is_file());
    assert!(repo.path().join(".cursor/tasks.json").is_file());

    let generic = json(&run(&[
        "editor-hook",
        root,
        "--editor",
        "generic",
        "--force",
    ]));
    assert_eq!(generic["editor"], "generic");
    assert!(repo.path().join(".callsieve/editor-hook.md").is_file());
    assert!(repo.path().join(".callsieve/editor-hook.json").is_file());
}

#[test]
fn daemon_background_records_state_and_stop_clears_running_status() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let output = Command::new(callsieve())
        .args(["daemon", root, "--background", "--interval-ms", "50"])
        .env("CALLSIEVE_TEST_BACKGROUND_NO_SPAWN", "1")
        .output()
        .expect("failed to run callsieve");
    let daemon = json(&output);
    assert_eq!(daemon["command"], "daemon");
    assert_eq!(daemon["state"]["mode"], "background");
    assert_eq!(daemon["state"]["pid"], 0);
    assert!(daemon["state"]["last_indexed_at"].as_u64().unwrap() > 0);
    assert!(repo.path().join(".callsieve/daemon.json").is_file());

    let stopped = json(&run(&["daemon-stop", root]));
    assert_eq!(stopped["state"]["status"], "stop_requested");
}

#[test]
fn trace_check_flags_grep_before_callsieve_context() {
    let repo = fixture_repo();
    let trace_path = repo.path().join("bad-trace.json");
    write(
        &trace_path,
        r#"{
  "tasks": [
    {
      "id": "bad-session",
      "task": "change auth",
      "session": {
        "callsieve": {
          "grep_commands": 2,
          "file_reads": 4,
          "tokens": 5000,
          "commands": ["rg createSession", "callsieve context . \"change auth\""]
        },
        "baseline": {
          "grep_commands": 4,
          "file_reads": 7,
          "tokens": 10000,
          "commands": ["rg createSession"]
        }
      }
    }
  ]
}"#,
    );

    let check = json(&run(&["trace-check", trace_path.to_str().unwrap()]));

    assert_eq!(check["status"], "fail");
    assert_eq!(check["violations"], 1);
    assert_eq!(check["grep_before_context"], 1);
    assert_eq!(check["grep_after_context"], 0);
    assert_eq!(check["context_first_compliant"], false);
    assert_eq!(
        check["violation_details"][0]["first_grep_command"],
        "rg createSession"
    );
}

#[test]
fn strict_trace_check_flags_file_reads_before_callsieve_context() {
    let repo = fixture_repo();
    let trace_path = repo.path().join("strict-bad-trace.json");
    write(
        &trace_path,
        r#"{
  "tasks": [
    {
      "id": "read-before-context",
      "task": "change auth",
      "session": {
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 2,
          "tokens": 5000,
          "commands": ["Get-Content src/auth/session.ts", "callsieve context . \"change auth\""]
        },
        "baseline": {
          "grep_commands": 4,
          "file_reads": 7,
          "tokens": 10000,
          "commands": ["rg createSession"]
        }
      }
    }
  ]
}"#,
    );

    let non_strict = json(&run(&["trace-check", trace_path.to_str().unwrap()]));
    assert_eq!(non_strict["status"], "pass");
    assert_eq!(non_strict["context_first_compliant"], true);

    let strict = json(&run(&[
        "trace-check",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));
    assert_eq!(strict["status"], "fail");
    assert_eq!(strict["strict"], true);
    assert_eq!(
        strict["violation_details"][0]["event_kind"],
        "read_before_context"
    );
    assert_eq!(
        strict["violation_details"][0]["first_file_read_command"],
        "Get-Content src/auth/session.ts"
    );
}

#[test]
fn trace_check_counts_grep_after_context_as_compliant() {
    let repo = fixture_repo();
    let trace_path = repo.path().join("good-trace.json");
    write(
        &trace_path,
        r#"{
  "tasks": [
    {
      "id": "good-session",
      "task": "change auth",
      "session": {
        "callsieve": {
          "grep_commands": 1,
          "file_reads": 2,
          "tokens": 3000,
          "commands": ["callsieve agent-context . \"change auth\"", "rg createSession"]
        }
      }
    }
  ]
}"#,
    );

    let check = json(&run(&["trace-check", trace_path.to_str().unwrap()]));

    assert_eq!(check["status"], "pass");
    assert_eq!(check["violations"], 0);
    assert_eq!(check["grep_before_context"], 0);
    assert_eq!(check["grep_after_context"], 1);
    assert_eq!(check["context_first_compliant"], true);
}

#[test]
fn benchmark_report_aggregates_two_local_repos() {
    let repo_a = fixture_repo();
    let root_a = repo_a.path().to_str().unwrap();
    json(&run(&["index", root_a]));
    let suite_a = repo_a.path().join("tasks-a.json");
    write(
        &suite_a,
        r#"{"tasks":[{"id":"auth-a","task":"change createSession token behavior","expected_files":["src/auth/session.ts","src/auth/token.ts"]}]}"#,
    );

    let repo_b = fixture_repo();
    let root_b = repo_b.path().to_str().unwrap();
    json(&run(&["index", root_b]));
    let suite_b = repo_b.path().join("tasks-b.json");
    write(
        &suite_b,
        r#"{"tasks":[{"id":"auth-b","task":"change tokenFor behavior","expected_files":["src/auth/token.ts"]}]}"#,
    );

    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("manifest.json");
    let escaped_root_a = root_a.replace('\\', "\\\\");
    let escaped_root_b = root_b.replace('\\', "\\\\");
    let escaped_suite_a = suite_a.to_string_lossy().replace('\\', "\\\\");
    let escaped_suite_b = suite_b.to_string_lossy().replace('\\', "\\\\");
    write(
        &manifest_path,
        &format!(
            r#"{{
  "repos": [
    {{"label":"repo-a","path":"{escaped_root_a}","suite_path":"{escaped_suite_a}"}},
    {{"label":"repo-b","path":"{escaped_root_b}","suite_path":"{escaped_suite_b}"}}
  ]
}}"#
        ),
    );

    let report = json(&run(&[
        "benchmark-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));

    assert_eq!(report["repo_count"], 2);
    assert_eq!(report["summary"]["repos"], 2);
    assert_eq!(report["summary"]["tasks"], 2);
    assert_eq!(report["summary"]["expected_files"], 3);
    assert_eq!(report["summary"]["expected_files_found"], 3);
    assert_eq!(report["summary"]["expected_file_recall"], 1.0);
    assert!(
        report["summary"]["total_estimated_token_savings"]
            .as_i64()
            .is_some()
    );
    let context_payload = &report["summary"]["context_payload_reduction"];
    assert_eq!(context_payload["label"], "context_payload_reduction");
    assert_eq!(context_payload["evidence_tier"], "platform_neutral_proxy");
    assert_eq!(context_payload["platform_scope"], "agent_platform_neutral");
    assert_eq!(
        context_payload["retrieval_cost"]["retrieval_model_tokens"],
        0
    );
    assert!(
        context_payload["warning"]
            .as_str()
            .unwrap()
            .contains("not observed whole-session token savings")
    );
    assert_eq!(
        report["summary"]["baseline_context_payload_tokens_estimate"],
        context_payload["baseline_context_payload_tokens_estimate"]
    );
    assert_eq!(
        report["summary"]["callsieve_context_payload_tokens_estimate"],
        context_payload["callsieve_context_payload_tokens_estimate"]
    );
    assert!(
        report["summary"]["total_avoided_grep_commands"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        report["repos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|repo| repo["label"] == "repo-a")
    );

    let doctor = json(&run(&["benchmark-doctor", manifest_path.to_str().unwrap()]));
    assert_eq!(doctor["status"], "pass");
    assert_eq!(doctor["repos"], 2);
}

#[test]
fn pilot_report_combines_benchmarks_traces_status_and_thresholds() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let suite = repo.path().join("tasks.json");
    write(
        &suite,
        r#"{"tasks":[{"id":"auth","task":"change createSession token behavior","expected_files":["src/auth/session.ts","src/auth/token.ts"]}]}"#,
    );
    let trace = repo.path().join("trace.json");
    write(
        &trace,
        r#"{
  "tasks": [
    {
      "id": "auth",
      "task": "change createSession token behavior",
      "expected_files": ["src/auth/session.ts"],
      "session": {
        "baseline": {
          "grep_commands": 5,
          "file_reads": 8,
          "tokens": 10000,
          "commands": ["rg createSession"],
          "files_read": ["src/auth/session.ts"]
        },
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 2,
          "tokens": 3000,
          "commands": ["callsieve agent-context . \"change auth\"", "rg token"],
          "files_read": ["src/auth/session.ts"]
        }
      }
    }
  ]
}"#,
    );
    let manifest_root = tempfile::tempdir().unwrap();
    let manifest_path = manifest_root.path().join("pilot.json");
    let escaped_root = root.replace('\\', "\\\\");
    let escaped_suite = suite.to_string_lossy().replace('\\', "\\\\");
    let escaped_trace = trace.to_string_lossy().replace('\\', "\\\\");
    write(
        &manifest_path,
        &format!(
            r#"{{
  "thresholds": {{
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": -300.0,
    "maximum_trace_violations": 0,
    "require_fresh_index": true
  }},
  "repos": [
    {{
      "label": "fixture",
      "path": "{escaped_root}",
      "languages": ["typescript", "python", "rust"],
      "suite_paths": ["{escaped_suite}"],
      "trace_paths": ["{escaped_trace}"]
    }}
  ]
}}"#
        ),
    );

    let report = json(&run(&[
        "pilot-report",
        manifest_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));

    assert_eq!(report["command"], "pilot-report");
    assert_eq!(
        report["status"],
        "pass",
        "{}",
        serde_json::to_string_pretty(&report).unwrap()
    );
    assert_eq!(report["proof"]["repos"], 1);
    assert_eq!(report["proof"]["sessions"], 1);
    assert_eq!(report["proof"]["trace_policy_violations"], 0);
    assert_eq!(report["proof"]["fresh_indexes"], 1);
    assert_eq!(report["benchmark"]["summary"]["expected_file_recall"], 1.0);

    let doctor = json(&run(&["pilot-doctor", manifest_path.to_str().unwrap()]));
    assert_eq!(doctor["status"], "pass");

    let evidence = json(&run(&[
        "evidence-pack",
        manifest_path.to_str().unwrap(),
        "--anonymize",
        "--limit",
        "5",
    ]));
    assert_eq!(evidence["command"], "evidence-pack");
    assert_eq!(evidence["anonymized"], true);
    assert_eq!(evidence["evidence"]["status"], "pass");
    assert_eq!(evidence["evidence"]["repos"][0]["path"], "<redacted>");
    assert_eq!(
        evidence["evidence"]["benchmark"]["repos"][0]["suite_paths"][0],
        "<redacted>"
    );
    assert_eq!(
        evidence["evidence"]["benchmark"]["repos"][0]["trace_paths"][0],
        "<redacted>"
    );
}

#[test]
fn status_and_watch_report_fresh_index_state() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let missing = json(&run(&["status", root]));
    assert_eq!(missing["index_exists"], false);
    assert_eq!(missing["fresh"], false);

    let watched = json(&run(&["watch", root]));
    assert_eq!(watched["command"], "watch");
    assert_eq!(watched["status"]["index_exists"], true);
    assert_eq!(watched["status"]["fresh"], true);
    assert_eq!(watched["status"]["watch_status"], "refreshed");
    assert_eq!(watched["status"]["watcher_mode"], "single_refresh");
    assert_eq!(watched["status"]["index_generation"], 1);
    assert_eq!(watched["status"]["changed_files"], 0);
    assert_eq!(watched["status"]["removed_files"], 0);
    assert_eq!(watched["status"]["lsp_enriched"], false);

    let status = json(&run(&["status", root]));
    assert_eq!(status["index_exists"], true);
    assert_eq!(status["fresh"], true);
    assert!(status["indexed_at"].as_u64().unwrap() > 0);
    assert!(status["index_age_seconds"].as_u64().is_some());
    assert_eq!(status["lsp_enriched"], false);
    assert!(status["lsp_servers"].as_array().is_some());
}

#[test]
fn daemon_once_writes_daemon_state_and_refreshes_index() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let daemon = json(&run(&["daemon", root, "--once"]));
    assert_eq!(daemon["command"], "daemon");
    assert_eq!(daemon["state"]["status"], "indexed_once");
    assert_eq!(daemon["state"]["mode"], "once");
    assert!(repo.path().join(".callsieve/daemon.json").is_file());

    let status = json(&run(&["daemon-status", root]));
    assert_eq!(status["state"]["status"], "indexed_once");

    let repo_status = json(&run(&["status", root]));
    assert_eq!(repo_status["daemon"]["status"], "indexed_once");
    assert!(repo_status["daemon"]["last_indexed_at"].as_u64().unwrap() > 0);

    let stopped = json(&run(&["daemon-stop", root]));
    assert_eq!(stopped["state"]["status"], "stop_requested");
    assert!(repo.path().join(".callsieve/daemon.stop").is_file());
}

#[test]
fn setup_agent_generates_policy_files() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();

    let setup = json(&run(&["setup-agent", "codex", root]));

    assert_eq!(setup["client"], "codex");
    assert!(
        setup["first_required_command"]
            .as_str()
            .unwrap()
            .contains("callsieve agent-context")
    );
    assert!(
        setup["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == ".codex/config.toml")
    );
    let config = fs::read_to_string(repo.path().join(".codex/config.toml")).unwrap();
    assert!(config.contains("[mcp_servers.callsieve]"));
    assert!(!config.contains("command = \"callsieve\""));
    assert!(
        fs::read_to_string(repo.path().join(".codex/CALLSIEVE.md"))
            .unwrap()
            .contains("First command for every coding task")
    );
    assert!(
        fs::read_to_string(repo.path().join(".codex/CALLSIEVE.md"))
            .unwrap()
            .contains("callsieve_context")
    );
    assert!(
        fs::read_to_string(repo.path().join(".codex/CALLSIEVE.md"))
            .unwrap()
            .contains("retrieval_cost.retrieval_model_tokens = 0")
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "roo"]));
    assert_eq!(setup["client"], "roo");
    assert!(repo.path().join(".roo/mcp.json").is_file());
    assert!(
        fs::read_to_string(repo.path().join(".roo/rules/callsieve.md"))
            .unwrap()
            .contains("Grep only if the context packet is insufficient")
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "generic"]));
    assert_eq!(setup["client"], "generic");
    assert!(repo.path().join(".callsieve/mcp.json").is_file());
    assert!(repo.path().join(".callsieve/mcp.toml").is_file());
    assert!(
        serde_json::from_str::<Value>(
            &fs::read_to_string(repo.path().join(".callsieve/mcp.json")).unwrap()
        )
        .unwrap()["mcpServers"]["callsieve"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "mcp")
    );
    assert!(
        fs::read_to_string(repo.path().join(".callsieve/mcp.toml"))
            .unwrap()
            .contains("[mcp_servers.callsieve]")
    );
}

#[test]
fn six_client_agent_setup_generates_expected_files() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "copilot"]));
    assert_eq!(setup["client"], "copilot");
    assert!(repo.path().join(".github/copilot-mcp.json").is_file());
    assert!(
        repo.path()
            .join(".github/copilot-instructions.md")
            .is_file()
    );
    assert!(
        repo.path()
            .join(".github/agents/callsieve-context.agent.md")
            .is_file()
    );

    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path().join("opencode.json"),
        r#"{"theme":"dark","instructions":["README.md"]}"#,
    );
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&[
        "agent-setup",
        root,
        "--client",
        "opencode",
        "--force",
    ]));
    assert_eq!(setup["client"], "opencode");
    assert!(repo.path().join(".opencode/CALLSIEVE.md").is_file());
    let opencode: Value =
        serde_json::from_slice(&fs::read(repo.path().join("opencode.json")).unwrap()).unwrap();
    assert_eq!(opencode["theme"], "dark");
    assert!(
        opencode["mcp"]["callsieve"]["command"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "mcp")
    );
    assert!(
        opencode["instructions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry == ".opencode/CALLSIEVE.md")
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "antigravity"]));
    assert_eq!(setup["client"], "antigravity");
    assert!(repo.path().join(".agents/mcp_config.json").is_file());
    assert!(
        repo.path()
            .join(".agents/skills/callsieve-context.md")
            .is_file()
    );
    assert!(repo.path().join(".agents/rules/callsieve.md").is_file());

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "cline"]));
    assert_eq!(setup["client"], "cline");
    assert!(repo.path().join(".cline/mcp.json").is_file());
    assert!(repo.path().join(".cline/rules/callsieve.md").is_file());
    assert!(repo.path().join(".clinerules/callsieve.md").is_file());

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "zoo"]));
    assert_eq!(setup["client"], "zoo");
    assert!(repo.path().join(".roo/mcp.json").is_file());
    assert!(repo.path().join(".roo/rules/callsieve.md").is_file());
    assert!(repo.path().join(".roo/rules-code/callsieve.md").is_file());
    assert!(!repo.path().join(".roomodes").is_file());

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "roo", "--force"]));
    assert_eq!(setup["client"], "roo");
    assert!(repo.path().join(".roo/rules-code/callsieve.md").is_file());
    assert!(repo.path().join(".roomodes").is_file());
    assert!(
        setup["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("deprecated"))
    );
}

#[test]
fn next_client_agent_setup_generates_expected_files_and_preserves_json() {
    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path().join(".vscode/mcp.json"),
        r#"{"inputs":[],"servers":{"existing":{"command":"node"}}}"#,
    );
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&[
        "agent-setup",
        root,
        "--client",
        "vscode",
        "--force",
    ]));
    assert_eq!(setup["client"], "vscode");
    assert!(repo.path().join(".vscode/mcp.json").is_file());
    assert!(
        repo.path()
            .join(".github/copilot-instructions.md")
            .is_file()
    );
    let vscode: Value =
        serde_json::from_slice(&fs::read(repo.path().join(".vscode/mcp.json")).unwrap()).unwrap();
    assert!(vscode["inputs"].is_array());
    assert_eq!(vscode["servers"]["existing"]["command"], "node");
    assert_eq!(vscode["servers"]["callsieve"]["type"], "stdio");
    assert_eq!(vscode["servers"]["callsieve"]["args"][0], "mcp");
    assert!(
        !fs::read_to_string(repo.path().join(".github/copilot-instructions.md"))
            .unwrap()
            .contains(root)
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "windsurf"]));
    assert_eq!(setup["client"], "windsurf");
    assert!(repo.path().join(".windsurf/rules/callsieve.md").is_file());
    assert!(
        repo.path()
            .join(".callsieve/integrations/windsurf-mcp.json")
            .is_file()
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "continue"]));
    assert_eq!(setup["client"], "continue");
    let continue_yaml =
        fs::read_to_string(repo.path().join(".continue/mcpServers/callsieve.yaml")).unwrap();
    assert!(continue_yaml.contains("schema: v1"));
    assert!(continue_yaml.contains("mcpServers:"));
    assert!(continue_yaml.contains("      - \"mcp\""));
    assert!(repo.path().join(".continue/rules/callsieve.md").is_file());

    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path().join(".junie/mcp/mcp.json"),
        r#"{"mcpServers":{"Existing":{"command":"node"}},"custom":true}"#,
    );
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "junie", "--force"]));
    assert_eq!(setup["client"], "junie");
    let junie: Value =
        serde_json::from_slice(&fs::read(repo.path().join(".junie/mcp/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(junie["custom"], true);
    assert_eq!(junie["mcpServers"]["Existing"]["command"], "node");
    assert_eq!(junie["mcpServers"]["callsieve"]["args"][0], "mcp");
    assert!(repo.path().join(".junie/guidelines.md").is_file());
    assert!(
        !fs::read_to_string(repo.path().join(".junie/guidelines.md"))
            .unwrap()
            .contains(root)
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "jetbrains"]));
    assert_eq!(setup["client"], "jetbrains");
    assert!(
        repo.path()
            .join(".callsieve/integrations/jetbrains-mcp.json")
            .is_file()
    );
    assert!(
        setup["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("--client junie"))
    );

    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path().join(".agents/skills/existing/SKILL.md"),
        "# Existing Skill\n",
    );
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "amp"]));
    assert_eq!(setup["client"], "amp");
    assert!(
        repo.path()
            .join(".agents/skills/callsieve-context/SKILL.md")
            .is_file()
    );
    assert!(
        repo.path()
            .join(".agents/skills/callsieve-context/mcp.json")
            .is_file()
    );
    assert!(
        repo.path()
            .join(".agents/skills/existing/SKILL.md")
            .is_file()
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "goose"]));
    assert_eq!(setup["client"], "goose");
    assert!(
        repo.path()
            .join(".callsieve/integrations/goose-config.yaml")
            .is_file()
    );
    assert!(
        repo.path()
            .join(".callsieve/integrations/goose-deeplink.txt")
            .is_file()
    );

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "warp"]));
    assert_eq!(setup["client"], "warp");
    assert!(
        repo.path()
            .join(".callsieve/integrations/warp-mcp.json")
            .is_file()
    );
    assert!(
        repo.path()
            .join(".callsieve/integrations/warp-agent.yaml")
            .is_file()
    );
    assert!(
        setup["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("template-only"))
    );
}

#[test]
fn zed_setup_merges_valid_settings_and_preserves_invalid_settings() {
    let repo = tempfile::tempdir().unwrap();
    write(
        repo.path().join(".zed/settings.json"),
        r#"{"theme":"One Dark","context_servers":{"existing":{"command":"node"}}}"#,
    );
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "zed", "--force"]));
    assert_eq!(setup["client"], "zed");
    let settings: Value =
        serde_json::from_slice(&fs::read(repo.path().join(".zed/settings.json")).unwrap()).unwrap();
    assert_eq!(settings["theme"], "One Dark");
    assert_eq!(settings["context_servers"]["existing"]["command"], "node");
    assert_eq!(settings["context_servers"]["callsieve"]["args"][0], "mcp");
    assert!(
        !repo
            .path()
            .join(".callsieve/integrations/zed-settings.json")
            .is_file()
    );

    let repo = tempfile::tempdir().unwrap();
    let invalid = "{\n  // JSONC is not overwritten\n  \"theme\": \"One Dark\"\n}\n";
    write(repo.path().join(".zed/settings.json"), invalid);
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "zed"]));
    assert_eq!(setup["client"], "zed");
    assert_eq!(
        fs::read_to_string(repo.path().join(".zed/settings.json")).unwrap(),
        invalid
    );
    assert!(
        repo.path()
            .join(".callsieve/integrations/zed-settings.json")
            .is_file()
    );
    assert!(
        setup["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("not mergeable JSON"))
    );
}

#[test]
fn mcp_config_prints_portable_json_and_toml() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();

    let json_config = json(&run(&["mcp-config", root, "--format", "json"]));
    assert_eq!(json_config["command"], "mcp-config");
    assert_eq!(json_config["format"], "json");
    assert_eq!(
        json_config["config"]["mcpServers"]["callsieve"]["args"][0],
        "mcp"
    );

    let toml_config = json(&run(&["mcp-config", root, "--format", "toml"]));
    assert_eq!(toml_config["format"], "toml");
    assert!(
        toml_config["config_text"]
            .as_str()
            .unwrap()
            .contains("[mcp_servers.callsieve]")
    );
}

#[test]
fn mcp_registry_manifest_prints_and_writes_server_descriptor() {
    let manifest = json(&run(&["mcp-registry-manifest"]));
    assert_eq!(manifest["name"], "io.github.philipjohnbasile/callsieve");
    assert_eq!(manifest["title"], "CallSieve");
    assert_eq!(manifest["packages"][0]["transport"]["type"], "stdio");
    assert_eq!(manifest["packages"][0]["transport"]["args"][0], "mcp");
    assert_eq!(
        manifest["_meta"]["io.modelcontextprotocol.registry/publisher-provided"]["publishing"],
        "descriptor only; this command does not contact the network or publish"
    );

    let repo = tempfile::tempdir().unwrap();
    let out = repo.path().join("server.json");
    let written = json(&run(&[
        "mcp-registry-manifest",
        "--out",
        out.to_str().unwrap(),
    ]));
    assert!(out.is_file());
    let saved: Value = serde_json::from_slice(&fs::read(out).unwrap()).unwrap();
    assert_eq!(saved, written);
}

#[test]
fn enforce_audits_agent_policy_index_trace_and_optional_shim() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    json(&run(&["agent-setup", root, "--client", "generic"]));
    let trace_path = repo.path().join("trace.json");
    write(
        &trace_path,
        r#"{
  "tasks": [
    {
      "id": "auth",
      "task": "change auth",
      "session": {
        "baseline": {"grep_commands": 2, "file_reads": 4, "tokens": 1000},
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 1,
          "tokens": 500,
          "commands": ["callsieve agent-context . \"change auth\""]
        }
      }
    }
  ]
}"#,
    );

    let enforce = json(&run(&[
        "enforce",
        root,
        "--client",
        "generic",
        "--trace",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));

    assert_eq!(enforce["status"], "pass");
    assert!(
        enforce["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "shim_doctor" && check["status"] == "warn")
    );
}

#[test]
fn enforce_strict_fails_on_reads_before_context() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    json(&run(&["agent-setup", root, "--client", "generic"]));
    let trace_path = repo.path().join("bad-strict-trace.json");
    write(
        &trace_path,
        r#"{
  "tasks": [
    {
      "id": "bad",
      "task": "change auth",
      "session": {
        "baseline": {"grep_commands": 2, "file_reads": 4, "tokens": 1000},
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 1,
          "tokens": 500,
          "commands": ["Get-Content src/auth/session.ts", "callsieve agent-context . \"change auth\""]
        }
      }
    }
  ]
}"#,
    );

    let enforce = json(&run(&[
        "enforce",
        root,
        "--client",
        "generic",
        "--trace",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));

    assert_eq!(enforce["status"], "fail");
    assert!(
        enforce["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "trace_policy" && check["status"] == "fail")
    );
}

#[test]
fn policy_check_exits_nonzero_on_strict_violation() {
    let repo = fixture_repo();
    let trace_path = repo.path().join("bad-policy.json");
    write(
        &trace_path,
        r#"{
  "tasks": [
    {
      "id": "bad",
      "session": {
        "baseline": {"grep_commands": 2, "file_reads": 4, "tokens": 1000},
        "callsieve": {
          "grep_commands": 0,
          "file_reads": 1,
          "tokens": 500,
          "commands": ["cat src/auth/session.ts", "callsieve context . \"change auth\""]
        }
      }
    }
  ]
}"#,
    );

    let output = run(&["policy-check", trace_path.to_str().unwrap(), "--strict"]);

    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["command"], "policy-check");
    assert_eq!(value["check"]["status"], "fail");
}

#[test]
fn shim_install_doctor_and_uninstall_manage_local_wrappers() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();

    let install = json(&run(&["shim", "install", root]));
    assert_eq!(install["status"], "pass");
    assert!(repo.path().join(".callsieve/bin").is_dir());
    assert!(!install["files"].as_array().unwrap().is_empty());

    let doctor = json(&run(&["shim", "doctor", root]));
    assert_eq!(doctor["command"], "shim doctor");
    assert!(
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "path_contains_shim_dir")
    );

    let uninstall = json(&run(&["shim", "uninstall", root]));
    assert_eq!(uninstall["status"], "pass");
    assert!(!uninstall["files"].as_array().unwrap().is_empty());
}

#[test]
fn hook_install_doctor_and_uninstall_manage_repo_local_launchers() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let install = json(&run(&[
        "hook", "install", root, "--client", "generic", "--strict", "--force",
    ]));
    assert_eq!(install["command"], "hook install");
    assert_eq!(install["status"], "pass");
    assert!(repo.path().join(".callsieve/agent-launch.ps1").is_file());
    assert!(repo.path().join(".callsieve/agent-launch.sh").is_file());
    assert!(repo.path().join(".callsieve/bin").is_dir());
    assert!(repo.path().join(".callsieve/mcp.json").is_file());
    assert!(
        install["path_instruction"]
            .as_str()
            .unwrap()
            .contains(".callsieve")
    );

    let doctor = json(&run(&["hook", "doctor", root]));
    assert_eq!(doctor["command"], "hook doctor");
    assert_eq!(doctor["status"], "pass");
    assert!(doctor.get("shim").is_none());
    assert!(doctor.get("path_instruction").is_none());
    assert!(doctor.get("codex_hooks").is_none());
    assert!(
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "hook_launchers" && check["status"] == "pass")
    );
    assert!(doctor["checks"].as_array().unwrap().iter().any(|check| {
        check["check"] == "path_contains_shim_dir"
            && check["status"] == "pass"
            && check["message"]
                .as_str()
                .unwrap()
                .contains("hook launchers prepend")
    }));

    let uninstall = json(&run(&["hook", "uninstall", root]));
    assert_eq!(uninstall["command"], "hook uninstall");
    assert!(!repo.path().join(".callsieve/agent-launch.ps1").is_file());
    assert!(!repo.path().join(".callsieve/agent-launch.sh").is_file());
}

#[test]
fn codex_hooks_install_doctor_and_uninstall_manage_lifecycle_hooks() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let install = json(&run(&[
        "codex-hooks",
        "install",
        root,
        "--strict",
        "--force",
        "--limit",
        "6",
        "--snippets-per-file",
        "1",
    ]));
    assert_eq!(install["command"], "codex-hooks install");
    assert_eq!(install["status"], "pass");
    assert_eq!(install["profile"], "slim");
    assert!(repo.path().join(".codex/hooks.json").is_file());
    assert!(repo.path().join(".callsieve/codex-hooks").is_dir());

    let hooks: Value =
        serde_json::from_str(&fs::read_to_string(repo.path().join(".codex/hooks.json")).unwrap())
            .unwrap();
    for event in ["UserPromptSubmit", "PreToolUse", "PermissionRequest"] {
        assert!(
            hooks["hooks"][event].as_array().is_some(),
            "{event} hook should be installed"
        );
    }
    assert!(
        hooks["hooks"].get("PostToolUse").is_none(),
        "Codex PostToolUse is intentionally not installed"
    );
    assert!(
        hooks["hooks"].get("Stop").is_none(),
        "Codex Stop is intentionally not installed"
    );
    let hooks_text = fs::read_to_string(repo.path().join(".codex/hooks.json")).unwrap();
    assert!(hooks_text.contains("commandWindows"));
    assert!(hooks_text.contains("codex-hook"));
    assert!(hooks_text.contains("--strict"));

    let doctor = json(&run(&["codex-hooks", "doctor", root, "--strict"]));
    assert_eq!(doctor["status"], "pass");
    assert_eq!(doctor["profile"], "slim");
    assert_eq!(doctor["trust"]["status"], "manual_review_required");
    assert!(
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |check| check["check"] == "codex_hook_command_windows" && check["status"] == "pass"
            )
    );

    let trust_ack = json(&run(&["codex-hooks", "trust-ack", root]));
    assert_eq!(trust_ack["status"], "pass");
    assert_eq!(trust_ack["profile"], "slim");
    assert!(
        repo.path()
            .join(".callsieve/codex-hooks/trust-reviewed.json")
            .is_file()
    );
    let trusted_doctor = json(&run(&["codex-hooks", "doctor", root, "--strict"]));
    assert_eq!(trusted_doctor["trust"]["status"], "reviewed");
    assert!(
        trusted_doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["check"] == "codex_hook_trust_ack" && check["status"] == "pass" })
    );

    let smoke = json(&run(&[
        "codex-hooks",
        "doctor",
        root,
        "--strict",
        "--smoke",
    ]));
    assert_eq!(
        smoke["status"],
        "pass",
        "{}",
        serde_json::to_string_pretty(&smoke).unwrap()
    );
    assert!(smoke["checks"].as_array().unwrap().iter().any(|check| {
        check["check"] == "codex_hook_smoke:disabled_stop" && check["status"] == "pass"
    }));
    let smoke_leftovers = fs::read_dir(repo.path().join(".callsieve/codex-hooks"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("callsieve-smoke-"))
        })
        .count();
    assert_eq!(smoke_leftovers, 0);

    write(
        repo.path().join(".callsieve/codex-hooks/old.state.json"),
        r#"{"version":1,"session_id":"old","violation_seen":true,"stop_blocked":true}"#,
    );
    write(
        repo.path().join(".callsieve/codex-hooks/old.trace.json"),
        r#"{"events":[{"hook_event":"PostToolUse"}]}"#,
    );
    write(
        repo.path()
            .join(".callsieve/codex-hooks/post-smoke.state.json"),
        r#"{"version":1,"session_id":"post-smoke"}"#,
    );
    let fixed = json(&run(&["codex-hooks", "doctor", root, "--fix"]));
    assert_eq!(fixed["status"], "pass");
    assert!(fixed["fixes"].as_array().unwrap().len() >= 3);
    assert!(
        !repo
            .path()
            .join(".callsieve/codex-hooks/old.state.json")
            .exists()
    );
    assert!(
        !repo
            .path()
            .join(".callsieve/codex-hooks/post-smoke.state.json")
            .exists()
    );
    assert!(repo.path().join(".callsieve/codex-hooks/archive").is_dir());

    let uninstall = json(&run(&["codex-hooks", "uninstall", root]));
    assert_eq!(uninstall["status"], "pass");
    assert!(!repo.path().join(".codex/hooks.json").is_file());
}

#[test]
fn codex_hook_user_prompt_submit_injects_callsieve_context() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let input = r#"{
  "session_id": "hook-session",
  "turn_id": "turn-1",
  "prompt": "change createSession token handling"
}"#;
    let output = json(&run_with_stdin(
        &[
            "codex-hook",
            "user-prompt-submit",
            root,
            "--strict",
            "--limit",
            "6",
            "--snippets-per-file",
            "1",
        ],
        input,
    ));

    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("CallSieve context ready"));
    assert!(context.contains("Use broad search only if this packet is insufficient"));
    assert!(!context.contains("blocked until CallSieve context is established"));
    assert!(context.contains("src/auth/session.ts"));
    let trace_path = repo
        .path()
        .join(".callsieve/codex-hooks/hook-session.trace.json");
    assert!(trace_path.is_file());
    let trace: Value = serde_json::from_str(&fs::read_to_string(trace_path).unwrap()).unwrap();
    assert_eq!(trace["session"]["callsieve"]["file_reads"], 0);
    assert!(
        trace["session"]["callsieve"]["files_read"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        trace["session"]["callsieve"]["context_selected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );
}

#[test]
fn user_prompt_hooks_skip_low_signal_acknowledgements() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let codex = json(&run_with_stdin(
        &["codex-hook", "user-prompt-submit", root, "--strict"],
        r#"{
  "session_id": "codex-skip",
  "turn_id": "turn-1",
  "prompt": "i did it"
}"#,
    ));
    assert_eq!(
        codex["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert!(
        codex["hookSpecificOutput"]
            .get("additionalContext")
            .is_none()
    );
    assert!(
        !repo
            .path()
            .join(".callsieve/codex-hooks/codex-skip.trace.json")
            .is_file()
    );

    let denied = json(&run_with_stdin(
        &["codex-hook", "pre-tool-use", root, "--strict"],
        r#"{
  "session_id": "codex-skip",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#,
    ));
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");

    let claude = json(&run_with_stdin(
        &["claude-hook", "user-prompt-submit", root, "--strict"],
        r#"{
  "session_id": "claude-skip",
  "turn_id": "turn-1",
  "prompt": "i did it"
}"#,
    ));
    assert!(
        claude["hookSpecificOutput"]
            .get("additionalContext")
            .is_none()
    );
    assert!(
        !repo
            .path()
            .join(".callsieve/claude-hooks/claude-skip.trace.json")
            .is_file()
    );

    let copilot = json(&run_with_stdin(
        &["copilot-hook", "user-prompt-submit", root, "--strict"],
        r#"{
  "session_id": "copilot-skip",
  "turn_id": "turn-1",
  "prompt": "i did it"
}"#,
    ));
    assert!(
        copilot["hookSpecificOutput"]
            .get("additionalContext")
            .is_none()
    );
    assert!(
        !repo
            .path()
            .join(".callsieve/copilot-hooks/copilot-skip.trace.json")
            .is_file()
    );
}

#[test]
fn user_prompt_hooks_reuse_previous_task_for_anaphoric_followups() {
    let repo = tempfile::tempdir().unwrap();
    let root_path = repo.path();
    write(
        root_path.join("src/cli.rs"),
        "fn codex_hooks_doctor() {}\nfn codex_hook_pre_tool_use() {}\nfn hook_doctor() {}\n",
    );
    write(
        root_path.join("tests/cli.rs"),
        "fn codex_hooks_doctor_smoke_test() {}\n",
    );
    write(root_path.join("src/mcp.rs"), "fn unrelated() {}\n");
    let root = root_path.to_str().unwrap();
    json(&run(&["index", root]));

    let first = json(&run_with_stdin(
        &[
            "codex-hook",
            "user-prompt-submit",
            root,
            "--strict",
            "--limit",
            "3",
            "--snippets-per-file",
            "0",
        ],
        r#"{
  "session_id": "codex-followup",
  "turn_id": "turn-1",
  "prompt": "what is broken with the hooks doctor smoke"
}"#,
    ));
    assert!(
        first["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("src/cli.rs")
    );

    let second = json(&run_with_stdin(
        &[
            "codex-hook",
            "user-prompt-submit",
            root,
            "--strict",
            "--limit",
            "3",
            "--snippets-per-file",
            "0",
        ],
        r#"{
  "session_id": "codex-followup",
  "turn_id": "turn-2",
  "prompt": "fix 1-5"
}"#,
    ));
    let context = second["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("src/cli.rs"));
    assert!(context.contains("Follow-up: fix 1-5"));
}

#[test]
fn user_prompt_hooks_recover_cold_followup_from_task_memory() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    json(&run(&[
        "agent-context",
        root,
        "change createSession token behavior",
    ]));

    let output = json(&run_with_stdin(
        &[
            "codex-hook",
            "user-prompt-submit",
            root,
            "--strict",
            "--limit",
            "6",
            "--snippets-per-file",
            "0",
        ],
        r#"{
  "session_id": "codex-cold-followup",
  "turn_id": "turn-1",
  "prompt": "do it"
}"#,
    ));
    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("src/auth/session.ts"));
    assert!(context.contains("Follow-up: do it"));
}

#[test]
fn codex_pre_tool_hook_blocks_broad_search_before_context() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let input = r#"{
  "session_id": "pre-deny",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#;
    let output = json(&run_with_stdin(
        &["codex-hook", "pre-tool-use", root, "--strict"],
        input,
    ));

    assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_no_codex_unsupported_top_level(&output);
    assert_eq!(
        output["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap(),
        "CallSieve needs context before broad repo search. Read the context packet first, or run callsieve agent-context, then retry if needed."
    );
}

#[test]
fn codex_pre_tool_hook_allows_callsieve_context_command() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let input = format!(
        r#"{{
  "session_id": "pre-allow",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": {{ "command": "callsieve agent-context {} \"change auth\"" }}
}}"#,
        root.replace('\\', "\\\\")
    );
    let output = json(&run_with_stdin(
        &["codex-hook", "pre-tool-use", root, "--strict"],
        &input,
    ));

    assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_no_codex_unsupported_top_level(&output);
    assert!(
        output["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none()
    );
    assert!(
        output["hookSpecificOutput"]
            .get("permissionDecisionReason")
            .is_none()
    );
}

#[test]
fn codex_permission_request_deny_omits_unsupported_suppress_output() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let output = json(&run_with_stdin(
        &["codex-hook", "permission-request", root, "--strict"],
        r#"{
  "session_id": "permission-deny",
  "hook_event_name": "PermissionRequest",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#,
    ));

    assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_no_codex_unsupported_top_level(&output);
    assert_eq!(
        output["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap(),
        "CallSieve needs context before broad repo search. Read the context packet first, or run callsieve agent-context, then retry if needed."
    );
}

#[test]
fn codex_permission_request_noop_omits_unsupported_suppress_output() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let output = json(&run_with_stdin(
        &["codex-hook", "permission-request", root, "--strict"],
        r#"{
  "session_id": "permission-noop",
  "hook_event_name": "PermissionRequest",
  "tool_name": "Bash",
  "tool_input": { "command": "git status --short" }
}"#,
    ));

    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_no_codex_unsupported_top_level(&output);
    assert!(
        output["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none()
    );
    assert!(
        output["hookSpecificOutput"]
            .get("permissionDecisionReason")
            .is_none()
    );
}

#[test]
fn codex_post_tool_hook_is_silent_noop() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let output = run_with_stdin(
        &["codex-hook", "post-tool-use", root, "--strict"],
        r#"{
  "session_id": "post-disabled",
  "hook_event_name": "PostToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "sed -n '1,20p' src/cli.rs" }
}"#,
    );

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert!(
        !repo
            .path()
            .join(".callsieve/codex-hooks/post-disabled.trace.json")
            .is_file()
    );
}

#[test]
fn codex_pre_tool_hook_strict_blocks_file_reads_but_allows_policy_reads() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let denied = json(&run_with_stdin(
        &["codex-hook", "pre-tool-use", root, "--strict"],
        r#"{
  "session_id": "strict-read-deny",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "Get-Content src\\auth\\session.ts" }
}"#,
    ));
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        denied["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap(),
        "CallSieve needs context before file reads in strict mode. Read the context packet first, or run callsieve agent-context, then retry if needed."
    );
    assert_no_codex_unsupported_top_level(&denied);

    let allowed = json(&run_with_stdin(
        &["codex-hook", "pre-tool-use", root, "--strict"],
        r#"{
  "session_id": "strict-read-allow",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "Get-Content AGENTS.md" }
}"#,
    ));
    assert_eq!(allowed["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert!(
        allowed["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none()
    );
    assert!(
        allowed["hookSpecificOutput"]
            .get("permissionDecisionReason")
            .is_none()
    );
    assert_no_codex_unsupported_top_level(&allowed);
}

#[test]
fn codex_stop_hook_is_silent_noop() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let violation = r#"{
  "session_id": "stop-after-violation",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#;
    let pre = json(&run_with_stdin(
        &["codex-hook", "pre-tool-use", root, "--strict"],
        violation,
    ));
    assert_eq!(pre["hookSpecificOutput"]["permissionDecision"], "deny");

    let output = run_with_stdin(
        &["codex-hook", "stop", root, "--strict"],
        r#"{
  "session_id": "stop-after-violation",
  "hook_event_name": "Stop",
  "stop_hook_active": false
}"#,
    );
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn claude_stop_hook_is_quiet_after_strict_violation() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let violation = r#"{
  "session_id": "claude-stop-after-violation",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#;
    let pre = json(&run_with_stdin(
        &["claude-hook", "pre-tool-use", root, "--strict"],
        violation,
    ));
    assert_eq!(pre["hookSpecificOutput"]["permissionDecision"], "deny");

    let stop = json(&run_with_stdin(
        &["claude-hook", "stop", root, "--strict"],
        r#"{
  "session_id": "claude-stop-after-violation",
  "hook_event_name": "Stop",
  "stop_hook_active": false
}"#,
    ));
    assert_eq!(stop["suppressOutput"], true);
    assert_eq!(stop["hookSpecificOutput"]["hookEventName"], "Stop");
    assert!(stop.get("decision").is_none());
    assert!(stop.get("reason").is_none());
}

#[test]
fn client_stop_hooks_are_quiet_after_strict_violation() {
    for client in ["copilot", "opencode", "antigravity", "cline"] {
        let repo = fixture_repo();
        let root = repo.path().to_str().unwrap();
        let session_id = format!("{client}-stop-after-violation");
        let violation = format!(
            r#"{{
  "session_id": "{session_id}",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": {{ "command": "rg createSession" }}
}}"#
        );
        let pre = json(&run_with_stdin(
            &[&format!("{client}-hook"), "pre-tool-use", root, "--strict"],
            &violation,
        ));
        assert_eq!(
            pre["hookSpecificOutput"]["permissionDecision"], "deny",
            "{client}"
        );

        let stop_input = format!(
            r#"{{
  "session_id": "{session_id}",
  "hook_event_name": "Stop",
  "stop_hook_active": false
}}"#
        );
        let stop = json(&run_with_stdin(
            &[&format!("{client}-hook"), "stop", root, "--strict"],
            &stop_input,
        ));
        assert_eq!(stop["suppressOutput"], true, "{client}");
        assert_eq!(
            stop["hookSpecificOutput"]["hookEventName"], "Stop",
            "{client}"
        );
        assert!(stop.get("decision").is_none(), "{client}");
        assert!(stop.get("reason").is_none(), "{client}");
    }
}

#[test]
fn codex_hook_install_and_enforce_require_lifecycle_hooks() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let install = json(&run(&[
        "hook", "install", root, "--client", "codex", "--strict", "--force",
    ]));
    assert_eq!(install["status"], "pass");
    assert!(
        install["codex_hooks"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file.as_str().unwrap().contains(".codex"))
    );
    assert!(repo.path().join(".codex/hooks.json").is_file());

    let enforce = json(&run(&["enforce", root, "--client", "codex", "--strict"]));
    assert_eq!(enforce["status"], "pass");
    assert!(
        enforce["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["check"] == "codex_hooks" && check["status"] == "pass" })
    );

    let doctor = json(&run(&["hook", "doctor", root]));
    assert_eq!(doctor["status"], "pass");
    assert!(
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["check"] == "codex_hooks" && check["status"] == "pass" })
    );
    assert_eq!(doctor["integrations"][0]["client"], "codex");
    assert_eq!(doctor["integrations"][0]["status"], "pass");
    assert_eq!(doctor["integrations"][0]["profile"], "slim");
    assert_eq!(
        doctor["integrations"][0]["events"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(doctor.get("codex_hooks").is_none());
    assert!(doctor.get("shim").is_none());

    fs::remove_file(repo.path().join(".codex/hooks.json")).unwrap();
    let failed = json(&run(&["enforce", root, "--client", "codex", "--strict"]));
    assert_eq!(failed["status"], "fail");
    assert!(
        failed["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["check"] == "codex_hooks" && check["status"] == "fail" })
    );
}

#[test]
fn claude_hooks_install_doctor_and_uninstall_manage_lifecycle_hooks() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    write(
        repo.path().join(".claude/settings.local.json"),
        r#"{
  "permissions": {
    "allow": ["Bash(git status)"]
  }
}"#,
    );

    let install = json(&run(&[
        "claude-hooks",
        "install",
        root,
        "--strict",
        "--force",
        "--limit",
        "6",
        "--snippets-per-file",
        "1",
    ]));
    assert_eq!(install["command"], "claude-hooks install");
    assert_eq!(install["status"], "pass");
    assert!(repo.path().join(".claude/settings.local.json").is_file());
    assert!(repo.path().join(".callsieve/claude-hooks").is_dir());

    let settings_text =
        fs::read_to_string(repo.path().join(".claude/settings.local.json")).unwrap();
    assert!(settings_text.contains("claude-hook"));
    assert!(settings_text.contains("\"args\""));
    assert!(settings_text.contains("--strict"));
    assert!(settings_text.contains("Bash(git status)"));
    let settings: Value = serde_json::from_str(&settings_text).unwrap();
    for event in [
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
        "Stop",
    ] {
        assert!(
            settings["hooks"][event].as_array().is_some(),
            "{event} hook should be installed"
        );
    }

    let doctor = json(&run(&["claude-hooks", "doctor", root, "--strict"]));
    assert_eq!(doctor["status"], "pass");
    assert!(
        doctor["checks"].as_array().unwrap().iter().any(|check| {
            check["check"] == "claude_hook_exec_form" && check["status"] == "pass"
        })
    );

    let uninstall = json(&run(&["claude-hooks", "uninstall", root]));
    assert_eq!(uninstall["status"], "pass");
    let remaining_text =
        fs::read_to_string(repo.path().join(".claude/settings.local.json")).unwrap();
    assert!(!remaining_text.contains("claude-hook"));
    assert!(remaining_text.contains("Bash(git status)"));
}

#[test]
fn claude_hook_user_prompt_submit_injects_callsieve_context() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let input = r#"{
  "session_id": "claude-hook-session",
  "turn_id": "turn-1",
  "prompt": "change createSession token handling"
}"#;
    let output = json(&run_with_stdin(
        &[
            "claude-hook",
            "user-prompt-submit",
            root,
            "--strict",
            "--limit",
            "6",
            "--snippets-per-file",
            "1",
        ],
        input,
    ));

    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("CallSieve context ready for Claude"));
    assert!(context.contains("Use broad search only if this packet is insufficient"));
    assert!(!context.contains("blocked until CallSieve context is established"));
    assert!(context.contains("src/auth/session.ts"));
    let trace_path = repo
        .path()
        .join(".callsieve/claude-hooks/claude-hook-session.trace.json");
    assert!(trace_path.is_file());
    let trace: Value = serde_json::from_str(&fs::read_to_string(trace_path).unwrap()).unwrap();
    assert_eq!(trace["session"]["callsieve"]["file_reads"], 0);
    assert!(
        trace["session"]["callsieve"]["files_read"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        trace["session"]["callsieve"]["context_selected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );
}

#[test]
fn claude_pre_tool_hook_blocks_broad_search_before_context() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let bash = json(&run_with_stdin(
        &["claude-hook", "pre-tool-use", root, "--strict"],
        r#"{
  "session_id": "claude-pre-deny",
  "hook_event_name": "PreToolUse",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#,
    ));
    assert_eq!(bash["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(bash["suppressOutput"], true);
    assert_eq!(bash["hookSpecificOutput"]["hookEventName"], "PreToolUse");

    let grep = json(&run_with_stdin(
        &["claude-hook", "pre-tool-use", root, "--strict"],
        r#"{
  "session_id": "claude-grep-deny",
  "hook_event_name": "PreToolUse",
  "tool_name": "Grep",
  "tool_input": { "pattern": "createSession", "path": "src" }
}"#,
    ));
    assert_eq!(grep["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(grep["suppressOutput"], true);
    assert_eq!(grep["hookSpecificOutput"]["hookEventName"], "PreToolUse");
}

#[test]
fn claude_permission_request_uses_claude_decision_shape() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let output = json(&run_with_stdin(
        &["claude-hook", "permission-request", root, "--strict"],
        r#"{
  "session_id": "claude-permission-deny",
  "hook_event_name": "PermissionRequest",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#,
    ));

    assert_eq!(output["suppressOutput"], true);
    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_eq!(output["hookSpecificOutput"]["decision"]["behavior"], "deny");
    assert!(
        output["hookSpecificOutput"]
            .get("permissionDecision")
            .is_none()
    );
    assert!(
        output["hookSpecificOutput"]["decision"]["message"]
            .as_str()
            .unwrap()
            .contains("broad repo search")
    );
}

#[test]
fn claude_hook_install_enforce_and_uninstall_cover_hook_shim_and_mcp() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();

    let install = json(&run(&[
        "hook", "install", root, "--client", "claude", "--strict", "--force",
    ]));
    assert_eq!(install["status"], "pass");
    assert!(repo.path().join(".mcp.json").is_file());
    assert!(repo.path().join("CLAUDE.md").is_file());
    assert!(repo.path().join(".claude/settings.local.json").is_file());
    assert!(repo.path().join(".callsieve/bin").is_dir());
    assert!(
        install["claude_hooks"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file.as_str().unwrap().contains(".claude"))
    );

    let enforce = json(&run(&["enforce", root, "--client", "claude", "--strict"]));
    assert_eq!(enforce["status"], "pass");
    assert!(
        enforce["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["check"] == "claude_hooks" && check["status"] == "pass" })
    );

    let uninstall = json(&run(&["hook", "uninstall", root]));
    assert_eq!(uninstall["command"], "hook uninstall");
    assert!(!repo.path().join(".claude/settings.local.json").is_file());
    assert!(!repo.path().join(".callsieve/agent-launch.ps1").is_file());
}

#[test]
fn new_client_hook_install_enforce_and_uninstall_cover_required_surfaces() {
    for (client, hook_file, hook_check) in [
        ("copilot", ".github/hooks/callsieve.json", "copilot_hooks"),
        (
            "opencode",
            ".opencode/plugins/callsieve.js",
            "opencode_hooks",
        ),
        ("antigravity", ".agents/hooks.json", "antigravity_hooks"),
        ("cline", ".cline/hooks/callsieve.json", "cline_hooks"),
    ] {
        let repo = fixture_repo();
        let root = repo.path().to_str().unwrap();

        let install = json(&run(&[
            "hook", "install", root, "--client", client, "--strict", "--force",
        ]));
        assert_eq!(install["status"], "pass", "{client}");
        assert!(
            install["client_hooks"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file.as_str().unwrap().contains(hook_file))
        );
        assert!(repo.path().join(hook_file).is_file(), "{client}");
        assert!(
            repo.path()
                .join(format!(".callsieve/{client}-hooks"))
                .is_dir(),
            "{client}"
        );

        let enforce = json(&run(&["enforce", root, "--client", client, "--strict"]));
        assert_eq!(
            enforce["status"],
            "pass",
            "{client}: {}",
            serde_json::to_string_pretty(&enforce).unwrap()
        );
        assert!(
            enforce["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["check"] == hook_check && check["status"] == "pass")
        );

        fs::remove_file(repo.path().join(hook_file)).unwrap();
        let failed = json(&run(&["enforce", root, "--client", client, "--strict"]));
        assert_eq!(failed["status"], "fail", "{client}");
        assert!(
            failed["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["check"] == hook_check && check["status"] == "fail")
        );

        let uninstall = json(&run(&["hook", "uninstall", root]));
        assert_eq!(uninstall["command"], "hook uninstall");
        assert!(!repo.path().join(".callsieve/agent-launch.ps1").is_file());
    }
}

#[test]
fn new_client_hook_handlers_inject_context_block_search_and_trace() {
    for client in ["copilot", "opencode", "antigravity", "cline"] {
        let repo = fixture_repo();
        let root = repo.path().to_str().unwrap();
        json(&run(&["index", root]));

        let session_id = format!("{client}-hook-session");
        let prompt_input = format!(
            r#"{{
  "session_id": "{session_id}",
  "turn_id": "turn-1",
  "prompt": "change createSession token handling"
}}"#
        );
        let prompt = json(&run_with_stdin(
            &[
                &format!("{client}-hook"),
                "user-prompt-submit",
                root,
                "--strict",
                "--limit",
                "6",
                "--snippets-per-file",
                "1",
            ],
            &prompt_input,
        ));
        let context = prompt["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains("CallSieve"), "{client}");
        assert!(
            context.contains("Use broad search only if this packet is insufficient"),
            "{client}"
        );
        assert!(
            !context.contains("blocked until CallSieve context is established"),
            "{client}"
        );
        assert!(context.contains("src/auth/session.ts"), "{client}");
        let trace_path = repo
            .path()
            .join(format!(".callsieve/{client}-hooks/{session_id}.trace.json"));
        assert!(trace_path.is_file(), "{client}");
        let trace: Value = serde_json::from_str(&fs::read_to_string(trace_path).unwrap()).unwrap();
        assert_eq!(trace["session"]["callsieve"]["file_reads"], 0, "{client}");
        assert!(
            trace["session"]["callsieve"]["files_read"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{client}"
        );
        assert!(
            trace["session"]["callsieve"]["context_selected_files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file == "src/auth/session.ts"),
            "{client}"
        );

        let deny_input = format!(
            r#"{{
  "session_id": "{client}-deny",
  "hook_event_name": "PreToolUse",
  "tool_name": "grep_search",
  "tool_input": {{ "pattern": "createSession", "path": "src" }}
}}"#
        );
        let denied = json(&run_with_stdin(
            &[&format!("{client}-hook"), "pre-tool-use", root, "--strict"],
            &deny_input,
        ));
        assert_eq!(
            denied["hookSpecificOutput"]["permissionDecision"], "deny",
            "{client}"
        );
        assert_eq!(denied["suppressOutput"], true, "{client}");
        assert_eq!(
            denied["hookSpecificOutput"]["hookEventName"], "PreToolUse",
            "{client}"
        );

        let allowed_input = format!(
            r#"{{
  "session_id": "{client}-allow",
  "hook_event_name": "PreToolUse",
  "tool_name": "read_file",
  "tool_input": {{ "path": "AGENTS.md" }}
}}"#
        );
        let allowed = json(&run_with_stdin(
            &[&format!("{client}-hook"), "pre-tool-use", root, "--strict"],
            &allowed_input,
        ));
        assert_eq!(
            allowed["hookSpecificOutput"]["permissionDecision"], "allow",
            "{client}"
        );
        assert_eq!(allowed["suppressOutput"], true, "{client}");
        assert_eq!(
            allowed["hookSpecificOutput"]["hookEventName"], "PreToolUse",
            "{client}"
        );
    }
}

#[test]
fn new_client_permission_request_preserves_suppressed_output() {
    for client in ["copilot", "opencode", "antigravity", "cline"] {
        let repo = fixture_repo();
        let root = repo.path().to_str().unwrap();

        let denied_input = r#"{
  "session_id": "client-permission-deny",
  "hook_event_name": "PermissionRequest",
  "tool_name": "Bash",
  "tool_input": { "command": "rg createSession" }
}"#;
        let denied = json(&run_with_stdin(
            &[
                &format!("{client}-hook"),
                "permission-request",
                root,
                "--strict",
            ],
            denied_input,
        ));
        assert_eq!(denied["suppressOutput"], true, "{client}");
        assert_eq!(
            denied["hookSpecificOutput"]["hookEventName"], "PermissionRequest",
            "{client}"
        );
        assert_eq!(
            denied["hookSpecificOutput"]["permissionDecision"], "deny",
            "{client}"
        );
        assert!(
            denied["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("broad repo search"),
            "{client}"
        );
    }
}

#[test]
fn cursor_and_zoo_strict_do_not_require_lifecycle_hooks() {
    for (client, forbidden_check) in [("cursor", "cursor_hooks"), ("zoo", "zoo_hooks")] {
        let repo = fixture_repo();
        let root = repo.path().to_str().unwrap();
        let install = json(&run(&[
            "hook", "install", root, "--client", client, "--strict", "--force",
        ]));
        assert_eq!(install["status"], "pass", "{client}");
        assert!(install.get("client_hooks").is_none(), "{client}");

        let enforce = json(&run(&["enforce", root, "--client", client, "--strict"]));
        assert_eq!(
            enforce["status"],
            "pass",
            "{client}: {}",
            serde_json::to_string_pretty(&enforce).unwrap()
        );
        assert!(
            !enforce["checks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|check| check["check"] == forbidden_check)
        );
    }
}

#[test]
fn next_clients_strict_require_setup_index_and_shims_but_not_lifecycle_hooks() {
    for (client, required_file) in [
        ("vscode", ".vscode/mcp.json"),
        ("windsurf", ".callsieve/integrations/windsurf-mcp.json"),
        ("continue", ".continue/mcpServers/callsieve.yaml"),
        ("zed", ".zed/settings.json"),
        ("junie", ".junie/mcp/mcp.json"),
        ("jetbrains", ".callsieve/integrations/jetbrains-mcp.json"),
        ("amp", ".agents/skills/callsieve-context/SKILL.md"),
        ("goose", ".callsieve/integrations/goose-config.yaml"),
        ("warp", ".callsieve/integrations/warp-mcp.json"),
    ] {
        let repo = fixture_repo();
        let root = repo.path().to_str().unwrap();
        let install = json(&run(&[
            "hook", "install", root, "--client", client, "--strict", "--force",
        ]));
        assert_eq!(install["status"], "pass", "{client}");
        assert!(install.get("client_hooks").is_none(), "{client}");
        assert!(install.get("codex_hooks").is_none(), "{client}");
        assert!(install.get("claude_hooks").is_none(), "{client}");
        assert!(repo.path().join(required_file).is_file(), "{client}");

        let enforce = json(&run(&["enforce", root, "--client", client, "--strict"]));
        assert_eq!(
            enforce["status"],
            "pass",
            "{client}: {}",
            serde_json::to_string_pretty(&enforce).unwrap()
        );
        assert!(
            !enforce["checks"].as_array().unwrap().iter().any(|check| {
                check["check"]
                    .as_str()
                    .unwrap_or_default()
                    .ends_with("_hooks")
            }),
            "{client}"
        );

        fs::remove_file(repo.path().join(required_file)).unwrap();
        let failed = json(&run(&["enforce", root, "--client", client, "--strict"]));
        assert_eq!(failed["status"], "fail", "{client}");
        assert!(
            failed["checks"].as_array().unwrap().iter().any(|check| {
                check["check"]
                    .as_str()
                    .unwrap_or_default()
                    .ends_with(required_file)
                    && check["status"] == "fail"
            }),
            "{client}: {}",
            serde_json::to_string_pretty(&failed).unwrap()
        );
    }
}

#[test]
fn strict_shim_trace_records_grep_before_context_violation() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let install = json(&run(&["shim", "install", root, "--strict"]));
    assert_eq!(install["strict"], true);
    let shim_file = if cfg!(windows) {
        repo.path().join(".callsieve/bin/rg.cmd")
    } else {
        repo.path().join(".callsieve/bin/rg")
    };
    assert!(fs::read_to_string(shim_file).unwrap().contains("shim-run"));

    let output = json(&run(&[
        "grep",
        root,
        "createSession",
        "--shim-strict",
        "--shim-command",
        "rg createSession",
    ]));
    assert_eq!(output["shim_event"]["policy_violation"], true);

    let trace_path = repo.path().join(".callsieve/shim-trace.json");
    assert!(trace_path.is_file());
    let check = json(&run(&[
        "trace-check",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));
    assert_eq!(check["status"], "fail");
    assert_eq!(
        check["violation_details"][0]["event_kind"],
        "grep_before_context"
    );
}

#[test]
fn shim_run_extracts_pattern_and_returns_context_before_passthrough() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    json(&run(&["shim", "install", root, "--strict"]));

    let output = json(&run(&[
        "shim-run",
        root,
        "--tool",
        "rg",
        "--strict",
        "--",
        "-n",
        "createSession",
        "src",
    ]));

    assert_eq!(output["command"], "shim-run");
    assert_eq!(output["tool"], "rg");
    assert_eq!(output["pattern"], "createSession");
    assert_eq!(
        output["context"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
    assert_eq!(output["shim_event"]["policy_violation"], true);
    assert!(repo.path().join(".callsieve/shim-trace.json").is_file());
}

#[test]
fn guard_returns_context_and_writes_trace_stub() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let trace_path = repo.path().join("guard-trace.json");

    let output = json(&run(&[
        "guard",
        root,
        "change createSession token behavior",
        "--trace-out",
        trace_path.to_str().unwrap(),
    ]));

    assert_eq!(output["command"], "guard");
    assert_eq!(output["trace_event"]["tool"], "callsieve_guard");
    assert_eq!(output["trace_event"]["context_first"], true);
    assert_eq!(
        output["context"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
    assert!(trace_path.is_file());

    let check = json(&run(&[
        "trace-check",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));
    assert_eq!(check["status"], "pass");
}

#[test]
fn begin_returns_context_and_writes_clean_trace_event() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let trace_path = repo.path().join("begin-trace.json");

    let output = json(&run(&[
        "begin",
        root,
        "change createSession token behavior",
        "--client",
        "generic",
        "--trace-out",
        trace_path.to_str().unwrap(),
    ]));

    assert_eq!(output["command"], "begin");
    assert_eq!(output["trace_event"]["classification"], "callsieve_context");
    assert!(
        output["trace_event"]["files_read"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        output["trace_event"]["context_selected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );
    assert_eq!(
        output["context"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
    assert!(trace_path.is_file());
    let trace: Value = serde_json::from_str(&fs::read_to_string(&trace_path).unwrap()).unwrap();
    assert_eq!(trace["session"]["callsieve"]["file_reads"], 0);
    assert!(
        trace["session"]["callsieve"]["files_read"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        trace["session"]["callsieve"]["context_selected_files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file == "src/auth/session.ts")
    );

    let check = json(&run(&[
        "trace-check",
        trace_path.to_str().unwrap(),
        "--strict",
    ]));
    assert_eq!(check["status"], "pass");
}

#[test]
fn begin_proof_trace_labels_explicit_trace_source() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let trace_path = repo.path().join("proof-begin-trace.json");

    let output = json(&run(&[
        "begin",
        root,
        "change createSession token behavior",
        "--client",
        "codex",
        "--trace-out",
        trace_path.to_str().unwrap(),
        "--proof-trace",
    ]));

    assert_eq!(output["command"], "begin");
    assert!(
        output["next_step"]
            .as_str()
            .unwrap()
            .contains("append explicit session-event records")
    );
    let trace: Value = serde_json::from_str(&fs::read_to_string(&trace_path).unwrap()).unwrap();
    assert_eq!(
        trace["metadata"]["proof_trace_source"],
        "explicit_callsieve_begin"
    );
    assert_eq!(trace["policy"]["proof_mode"], true);
    assert_eq!(trace["policy"]["post_tool_hook_required"], false);
    assert_eq!(trace["policy"]["event_source"], "explicit_session_events");
    assert!(
        output["proof_next_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command.as_str().unwrap().contains("--tokens"))
    );
    assert!(
        output["proof_next_commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command
                .as_str()
                .unwrap()
                .contains("--context-selected-file"))
    );

    let missing_tokens = run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "cat src/auth/session.ts",
        "--phase",
        "callsieve",
        "--files-read",
        "src/auth/session.ts",
    ]);
    assert!(!missing_tokens.status.success());
    assert!(
        String::from_utf8_lossy(&missing_tokens.stdout)
            .contains("proof-mode traces require --tokens")
    );

    let missing_phase = run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "cat src/auth/session.ts",
        "--tokens",
        "44",
        "--files-read",
        "src/auth/session.ts",
    ]);
    assert!(!missing_phase.status.success());
    assert!(
        String::from_utf8_lossy(&missing_phase.stdout)
            .contains("proof-mode traces require explicit --phase")
    );
}

#[test]
fn grep_wrapper_returns_context_before_rg() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let output = json(&run(&["grep", root, "createSession"]));

    assert_eq!(output["command"], "grep");
    assert!(output["rg"].is_null());
    assert!(
        output["rg_status"]
            .as_str()
            .unwrap()
            .contains("pass --run-rg")
    );
    assert_eq!(output["audit_event"]["tool"], "callsieve_grep");
    assert_eq!(output["audit_event"]["context_first"], true);
    assert_eq!(
        output["context"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
}

#[test]
fn benchmark_suite_reports_missed_expected_files() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let suite_path = repo.path().join("tasks-with-miss.json");
    write(
        &suite_path,
        r#"{
  "tasks": [
    {
      "id": "missing-auth-helper",
      "task": "change createSession token behavior",
      "expected_files": ["src/auth/session.ts", "src/auth/missing.ts"]
    }
  ]
}"#,
    );

    let suite = json(&run(&[
        "benchmark-suite",
        root,
        suite_path.to_str().unwrap(),
        "--limit",
        "5",
    ]));

    assert_eq!(suite["summary"]["tasks_with_misses"], 1);
    assert_eq!(suite["summary"]["missed_expected_files"], 1);
    assert_eq!(
        suite["summary"]["misses"][0]["missing_files"][0],
        "src/auth/missing.ts"
    );
    assert!(
        !suite["tasks"][0]["miss_reasons"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn context_no_snippets_omits_snippets() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let context = json(&run(&[
        "context",
        root,
        "change createSession token behavior",
        "--no-snippets",
    ]));

    assert!(context["read_first"][0].get("snippets").is_none());
}

#[test]
fn missing_index_returns_json_error() {
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();

    for args in [
        vec!["stats", root],
        vec!["query", root, "where is auth handled?"],
        vec!["context", root, "change createSession token behavior"],
        vec!["agent-context", root, "change createSession token behavior"],
        vec!["benchmark", root, "change createSession token behavior"],
        vec!["benchmark-suite", root, "tasks.json"],
        vec!["grep", root, "createSession"],
    ] {
        let output = run(&args);

        assert!(!output.status.success());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .contains("run `callsieve index")
        );
    }
}

#[test]
fn context_surfaces_codeowners_ownership_on_read_first_entries() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    write(
        repo.path().join(".github/CODEOWNERS"),
        "# CODEOWNERS for the fixture repo\n\
         *                       @everyone\n\
         *.ts                    @ts-team\n\
         src/auth/                @org/auth-team alice@example.com\n",
    );
    json(&run(&["index", root]));

    let context = json(&run(&[
        "context",
        root,
        "change createSession token behavior",
        "--limit",
        "5",
    ]));

    let read_first = context["read_first"].as_array().unwrap();
    let session_entry = read_first
        .iter()
        .find(|entry| entry["file"] == "src/auth/session.ts")
        .expect("session.ts should appear in read_first");

    let ownership = &session_entry["ownership"];
    assert!(
        ownership.is_object(),
        "ownership should be an object for files matching CODEOWNERS rules: {ownership}"
    );
    let teams: Vec<&str> = ownership["teams"]
        .as_array()
        .expect("teams should be an array")
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    let owners: Vec<&str> = ownership["owners"]
        .as_array()
        .expect("owners should be an array")
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    assert_eq!(teams, vec!["@org/auth-team"]);
    assert_eq!(owners, vec!["alice@example.com"]);
}

#[test]
fn context_omits_ownership_when_no_codeowners_file_is_present() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));

    let context = json(&run(&[
        "context",
        root,
        "change createSession token behavior",
        "--limit",
        "5",
    ]));

    for entry in context["read_first"].as_array().unwrap() {
        assert!(
            entry.get("ownership").is_none(),
            "ownership should be absent when no CODEOWNERS file exists, got {entry}",
        );
    }
}

#[test]
fn session_finish_ground_truth_metrics_hit() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let trace_path = repo.path().join("hit-session.json");
    let summary_path = repo.path().join("hit-summary.json");

    json(&run(&[
        "session-start",
        root,
        "change createSession token behavior",
        "--client",
        "claude",
        "--model",
        "claude-opus-4-7",
        "--trace",
        trace_path.to_str().unwrap(),
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "rg createSession",
        "--tokens",
        "5000",
        "--phase",
        "baseline",
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "callsieve agent-context . \"change createSession token behavior\"",
        "--files-read",
        "src/auth/session.ts",
        "--files-read",
        "src/auth/token.ts",
        "--tokens",
        "1500",
        "--phase",
        "callsieve",
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "Read",
        "--files-read",
        "src/auth/session.test.ts",
        "--tokens",
        "200",
        "--phase",
        "callsieve",
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "Edit",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "0",
        "--phase",
        "callsieve",
    ]));

    let finish = json(&run(&[
        "session-finish",
        trace_path.to_str().unwrap(),
        "--out",
        summary_path.to_str().unwrap(),
        "--ground-truth-files",
        "src/auth/session.ts",
    ]));

    assert_eq!(finish["command"], "session-finish");
    assert_eq!(finish["first_correct_file_rate_at_k"], 1.0);
    assert_eq!(finish["first_correct_file_rate_k"], 5);
    assert_eq!(finish["turns_to_first_edit"], 4);
    assert_eq!(finish["wrong_files_read"], 1);

    let summary_json: Value = serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
    assert_eq!(summary_json["first_correct_file_rate_at_k"], 1.0);
    assert_eq!(summary_json["first_correct_file_rate_k"], 5);
    assert_eq!(summary_json["turns_to_first_edit"], 4);
    assert_eq!(summary_json["wrong_files_read"], 1);
    assert_eq!(
        summary_json["ground_truth_files"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn session_finish_ground_truth_metrics_miss() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let trace_path = repo.path().join("miss-session.json");
    let summary_path = repo.path().join("miss-summary.json");

    json(&run(&[
        "session-start",
        root,
        "rename refreshSession",
        "--client",
        "claude",
        "--model",
        "claude-opus-4-7",
        "--trace",
        trace_path.to_str().unwrap(),
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "callsieve agent-context . \"rename refreshSession\"",
        "--files-read",
        "src/auth/session.ts",
        "--files-read",
        "src/auth/token.ts",
        "--tokens",
        "1200",
        "--phase",
        "callsieve",
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "Read",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "300",
        "--phase",
        "callsieve",
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "Read",
        "--files-read",
        "src/auth/token.ts",
        "--tokens",
        "200",
        "--phase",
        "callsieve",
    ]));

    let finish = json(&run(&[
        "session-finish",
        trace_path.to_str().unwrap(),
        "--out",
        summary_path.to_str().unwrap(),
        "--ground-truth-files",
        "src/auth/refresh.ts",
    ]));

    assert_eq!(finish["command"], "session-finish");
    assert_eq!(finish["first_correct_file_rate_at_k"], 0.0);
    assert_eq!(finish["first_correct_file_rate_k"], 5);
    assert!(finish["turns_to_first_edit"].is_null());
    assert_eq!(finish["wrong_files_read"], 2);

    let summary_json: Value = serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
    assert_eq!(summary_json["first_correct_file_rate_at_k"], 0.0);
    assert!(summary_json["turns_to_first_edit"].is_null());
    assert_eq!(summary_json["wrong_files_read"], 2);
}

#[test]
fn session_finish_without_ground_truth_omits_metrics() {
    let repo = fixture_repo();
    let root = repo.path().to_str().unwrap();
    json(&run(&["index", root]));
    let trace_path = repo.path().join("plain-session.json");
    let summary_path = repo.path().join("plain-summary.json");

    json(&run(&[
        "session-start",
        root,
        "noop",
        "--client",
        "claude",
        "--model",
        "claude-opus-4-7",
        "--trace",
        trace_path.to_str().unwrap(),
    ]));
    json(&run(&[
        "session-event",
        trace_path.to_str().unwrap(),
        "--command",
        "callsieve agent-context . \"noop\"",
        "--files-read",
        "src/auth/session.ts",
        "--tokens",
        "1000",
        "--phase",
        "callsieve",
    ]));

    let finish = json(&run(&[
        "session-finish",
        trace_path.to_str().unwrap(),
        "--out",
        summary_path.to_str().unwrap(),
    ]));
    assert_eq!(finish["command"], "session-finish");
    assert!(finish.get("first_correct_file_rate_at_k").is_none());
    assert!(finish.get("turns_to_first_edit").is_none());
    assert!(finish.get("wrong_files_read").is_none());

    let summary_json: Value = serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
    assert!(summary_json.get("first_correct_file_rate_at_k").is_none());
    assert!(summary_json.get("turns_to_first_edit").is_none());
    assert!(summary_json.get("wrong_files_read").is_none());
}
