use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
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
    assert!(repo.path().join(".callsieve/index.json").is_file());

    let stats = json(&run(&["stats", root]));
    assert_eq!(stats["files"], 5);
    assert_eq!(stats["tests"], 1);
    assert_eq!(stats["languages"]["typescript"], 3);
    assert_eq!(stats["languages"]["python"], 1);
    assert_eq!(stats["languages"]["rust"], 1);
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
    assert!(!first["why"].as_array().unwrap().is_empty());
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
