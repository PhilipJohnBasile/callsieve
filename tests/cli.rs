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
        output["context"]["read_first"][0]["file"],
        "src/auth/session.ts"
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
            .contains("Preferred first tool")
    );
    assert_eq!(responses[2]["result"]["isError"], false);
    assert_eq!(
        responses[2]["result"]["structuredContent"]["read_first"][0]["file"],
        "src/auth/session.ts"
    );
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
        "--files-read",
        "src/auth/session.ts",
        "--files-read",
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

    let repo = tempfile::tempdir().unwrap();
    let root = repo.path().to_str().unwrap();
    let setup = json(&run(&["agent-setup", root, "--client", "roo"]));
    assert_eq!(setup["client"], "roo");
    assert!(repo.path().join(".roo/mcp.json").is_file());
    assert!(
        fs::read_to_string(repo.path().join(".roo/rules/callsieve.md"))
            .unwrap()
            .contains("grep only if the context packet is insufficient")
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
    assert!(
        fs::read_to_string(shim_file)
            .unwrap()
            .contains("--shim-strict")
    );

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
