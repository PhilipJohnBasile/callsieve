#!/usr/bin/env python3
"""Run public agent-native baselines with Codex CLI.

This harness intentionally does not use CallSieve inside the Codex prompts.
CallSieve is used before and after the external run to prepare the template,
validate the filled log, standardize the baseline, and run public proof.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
BASE_PUBLIC_PROOF = REPO_ROOT / "benchmarks/public-proof-manifest.example.json"
RESULTS_DIR = REPO_ROOT / "benchmarks/public/results"
PROTOCOL_PATH = RESULTS_DIR / "agent-native-protocol.json"
SUITES: dict[str, dict[str, str]] = {
    "requests": {
        "label": "Requests",
        "repo": "psf/requests",
        "repo_path": "benchmarks/public/repos/psf/requests",
        "manifest": "benchmarks/public/manifest.json",
        "template": "benchmarks/public/results/agent-native-requests-template.json",
        "artifact_prefix": "codex-cli-requests",
        "baseline_id": "codex-cli-public-requests",
        "proof_manifest": "benchmarks/public-proof-manifest.codex-cli-requests.json",
        "measurement_note": "Measured pinned public Requests tasks with Codex CLI native shell search/read only.",
    },
    "rust": {
        "label": "CallSieve Rust",
        "repo": "PhilipJohnBasile/callsieve",
        "repo_path": "benchmarks/public/repos/PhilipJohnBasile/callsieve",
        "manifest": "benchmarks/public/manifest-rust.json",
        "template": "benchmarks/public/results/agent-native-rust-callsieve-template.json",
        "artifact_prefix": "codex-cli-rust-callsieve",
        "baseline_id": "codex-cli-public-rust-callsieve",
        "proof_manifest": "benchmarks/public-proof-manifest.codex-cli-rust-callsieve.json",
        "measurement_note": "Measured pinned public CallSieve Rust language-slice tasks with Codex CLI native shell search/read only.",
    },
    "typescript": {
        "label": "CallSieve TypeScript",
        "repo": "PhilipJohnBasile/callsieve",
        "repo_path": "benchmarks/public/repos/PhilipJohnBasile/callsieve",
        "manifest": "benchmarks/public/manifest-typescript.json",
        "template": "benchmarks/public/results/agent-native-typescript-callsieve-template.json",
        "artifact_prefix": "codex-cli-typescript-callsieve",
        "baseline_id": "codex-cli-public-typescript-callsieve",
        "proof_manifest": "benchmarks/public-proof-manifest.codex-cli-typescript-callsieve.json",
        "measurement_note": "Measured pinned public CallSieve TypeScript language-slice tasks with Codex CLI native shell search/read only.",
    },
}


class HarnessConfig:
    def __init__(self, suite: str) -> None:
        try:
            spec = SUITES[suite]
        except KeyError as exc:
            raise SystemExit(f"Unknown suite {suite!r}; choose one of {', '.join(SUITES)}") from exc
        self.suite = suite
        self.label = spec["label"]
        self.repo = spec["repo"]
        self.repo_path = REPO_ROOT / spec["repo_path"]
        self.manifest = REPO_ROOT / spec["manifest"]
        self.template_path = REPO_ROOT / spec["template"]
        self.artifact_prefix = spec["artifact_prefix"]
        self.baseline_id = spec["baseline_id"]
        self.overlay_manifest_path = REPO_ROOT / spec["proof_manifest"]
        self.measurement_note = spec["measurement_note"]
        self.schema_path = RESULTS_DIR / f"{self.artifact_prefix}-result-schema.json"
        self.task_log_path = RESULTS_DIR / f"{self.artifact_prefix}-task-log.json"
        self.transcript_path = RESULTS_DIR / f"{self.artifact_prefix}-transcript.json"
        self.check_path = RESULTS_DIR / f"{self.artifact_prefix}-check.json"
        self.baseline_path = RESULTS_DIR / f"{self.artifact_prefix}.json"
        self.proof_path = RESULTS_DIR / f"{self.artifact_prefix}-public-proof.json"
        self.raw_dir = RESULTS_DIR / f"{self.artifact_prefix}-raw"
        self.plan_path = RESULTS_DIR / f"{self.artifact_prefix}-measurement-plan.json"
        self.lock_path = RESULTS_DIR / f"{self.artifact_prefix}.lock"
        repo_lock_slug = self.repo.replace("/", "-")
        self.repo_lock_path = RESULTS_DIR / f"codex-cli-{repo_lock_slug}.repo.lock"


CONFIG = HarnessConfig("requests")
REQUESTS_REPO = CONFIG.repo_path
PUBLIC_MANIFEST = CONFIG.manifest
TEMPLATE_PATH = CONFIG.template_path
SCHEMA_PATH = CONFIG.schema_path
TASK_LOG_PATH = CONFIG.task_log_path
TRANSCRIPT_PATH = CONFIG.transcript_path
CHECK_PATH = CONFIG.check_path
BASELINE_PATH = CONFIG.baseline_path
OVERLAY_MANIFEST_PATH = CONFIG.overlay_manifest_path
PROOF_PATH = CONFIG.proof_path
RAW_DIR = CONFIG.raw_dir
PLAN_PATH = CONFIG.plan_path
LOCK_PATH = CONFIG.lock_path
REPO_LOCK_PATH = CONFIG.repo_lock_path


def configure_suite(suite: str) -> None:
    global CONFIG
    global REQUESTS_REPO
    global PUBLIC_MANIFEST
    global TEMPLATE_PATH
    global SCHEMA_PATH
    global TASK_LOG_PATH
    global TRANSCRIPT_PATH
    global CHECK_PATH
    global BASELINE_PATH
    global OVERLAY_MANIFEST_PATH
    global PROOF_PATH
    global RAW_DIR
    global PLAN_PATH
    global LOCK_PATH
    global REPO_LOCK_PATH

    CONFIG = HarnessConfig(suite)
    REQUESTS_REPO = CONFIG.repo_path
    PUBLIC_MANIFEST = CONFIG.manifest
    TEMPLATE_PATH = CONFIG.template_path
    SCHEMA_PATH = CONFIG.schema_path
    TASK_LOG_PATH = CONFIG.task_log_path
    TRANSCRIPT_PATH = CONFIG.transcript_path
    CHECK_PATH = CONFIG.check_path
    BASELINE_PATH = CONFIG.baseline_path
    OVERLAY_MANIFEST_PATH = CONFIG.overlay_manifest_path
    PROOF_PATH = CONFIG.proof_path
    RAW_DIR = CONFIG.raw_dir
    PLAN_PATH = CONFIG.plan_path
    LOCK_PATH = CONFIG.lock_path
    REPO_LOCK_PATH = CONFIG.repo_lock_path


RESULT_SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {
        "task_id": {"type": "string"},
        "files_selected_or_read": {
            "type": "array",
            "items": {"type": "string"},
            "minItems": 1,
        },
        "rationale": {"type": "string"},
    },
    "required": ["task_id", "files_selected_or_read", "rationale"],
    "additionalProperties": False,
}
DISABLED_CODEX_FEATURES = [
    "plugins",
    "memories",
    "chronicle",
    "apps",
    "browser_use",
    "browser_use_external",
    "computer_use",
    "multi_agent",
    "goals",
    "hooks",
]
LINE_SUFFIX = re.compile(r":\d+(?::\d+)?$")


class CommandError(RuntimeError):
    def __init__(self, cmd: list[str], cwd: Path, result: subprocess.CompletedProcess[str]):
        super().__init__(
            f"command failed ({result.returncode}) in {cwd}: {' '.join(cmd)}\n"
            f"stdout:\n{result.stdout}\n\nstderr:\n{result.stderr}"
        )


class HarnessLock:
    def __enter__(self) -> "HarnessLock":
        self.paths = [LOCK_PATH, REPO_LOCK_PATH]
        self.acquired: list[Path] = []
        LOCK_PATH.parent.mkdir(parents=True, exist_ok=True)
        try:
            for path in self.paths:
                fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                with os.fdopen(fd, "w", encoding="utf-8") as handle:
                    handle.write(f"pid={os.getpid()}\n")
                self.acquired.append(path)
        except FileExistsError as exc:
            self._release()
            raise SystemExit(
                f"{repo_relative(Path(exc.filename))} already exists. Another Codex measurement "
                "run may be active; remove the lock only after confirming no run is in progress."
            ) from exc
        return self

    def __exit__(self, exc_type: Any, exc: Any, traceback: Any) -> None:
        self._release()

    def _release(self) -> None:
        for path in reversed(getattr(self, "acquired", [])):
            try:
                path.unlink()
            except FileNotFoundError:
                pass
        self.acquired = []


def repo_relative(path: Path) -> str:
    return path.relative_to(REPO_ROOT).as_posix()


def run(cmd: list[str], cwd: Path = REPO_ROOT, check: bool = True) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        cmd,
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if check and result.returncode != 0:
        raise CommandError(cmd, cwd, result)
    return result


def load_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        json.dump(value, handle, indent=2)
        handle.write("\n")


def codex_bin() -> str:
    codex = shutil.which("codex")
    if codex is not None:
        return codex
    app_codex = Path("/Applications/Codex.app/Contents/Resources/codex")
    if app_codex.is_file():
        return str(app_codex)
    raise SystemExit("Codex CLI is not on PATH. Install or open Codex before measuring.")


def codex_version() -> str:
    result = run([codex_bin(), "--version"], check=True)
    return result.stdout.strip()


def require_codex_login() -> None:
    result = run([codex_bin(), "login", "status"], check=False)
    output = f"{result.stdout}\n{result.stderr}"
    if result.returncode != 0 or "Logged in" not in output:
        raise SystemExit(
            "Codex CLI is not logged in. Run `codex login`, then rerun this harness. "
            "Refusing to fabricate an agent-native baseline."
        )


def ensure_clean_requests_checkout() -> None:
    result = run(["git", "status", "--porcelain"], cwd=REQUESTS_REPO)
    for line in result.stdout.splitlines():
        path = line[3:].strip() if len(line) > 3 else line.strip()
        if path in {".callsieve", ".callsieve/"} or path.startswith(".callsieve/"):
            continue
        raise RuntimeError(
            f"{CONFIG.label} checkout has local changes at {path}; refusing pinned measurement checkout."
        )


def checkout_task_repo(task: dict[str, Any]) -> str | None:
    repo = task.get("repo")
    if repo not in {None, CONFIG.repo}:
        raise RuntimeError(f"task {task['id']} repo {repo!r} is not supported by this harness")
    base_commit = task.get("base_commit")
    if not base_commit:
        return None
    ensure_clean_requests_checkout()
    run(["git", "checkout", "-f", str(base_commit)], cwd=REQUESTS_REPO)
    head = run(["git", "rev-parse", "HEAD"], cwd=REQUESTS_REPO).stdout.strip()
    if head != base_commit:
        raise RuntimeError(f"task {task['id']} checkout ended at {head}, expected {base_commit}")
    return head


def current_requests_head() -> str:
    return run(["git", "rev-parse", "HEAD"], cwd=REQUESTS_REPO).stdout.strip()


def prepare_protocol() -> None:
    run(
        [
            "cargo",
            "run",
            "--",
            "agent-native-protocol",
            "--out",
            repo_relative(PROTOCOL_PATH),
        ]
    )


def prepare_template() -> dict[str, Any]:
    run(
        [
            "cargo",
            "run",
            "--",
            "agent-native-template",
            repo_relative(REQUESTS_REPO),
            repo_relative(PUBLIC_MANIFEST),
            "--k",
            "5",
            "--out",
            repo_relative(TEMPLATE_PATH),
        ]
    )
    return load_json(TEMPLATE_PATH)


def prompt_for_task(task: dict[str, Any]) -> str:
    prompt = (
        "Measurement task for CallSieve public proof.\n"
        "Do not use CallSieve, callsieve commands, callsieve MCP tools, or any repo-specific "
        "precomputed CallSieve context.\n"
        "Do not use browser, web, MCP, plugin, memory, or external documentation tools.\n"
        "Do not edit files.\n"
        "Use only Codex native read-only shell search and file-read commands inside this "
        f"{CONFIG.label} checkout.\n"
        "Inspect the repository as needed. You must read every file you rely on.\n"
        "Return JSON only, matching the provided schema.\n\n"
        f"Task id: {task['id']}\n"
        f"Repository task: {task['task']}\n"
        "\n"
        "Return files_selected_or_read as repo-relative paths ordered by first use."
    )
    assert_no_expected_file_leak(task, prompt)
    return prompt


def assert_no_expected_file_leak(task: dict[str, Any], prompt: str) -> None:
    for expected_file in task.get("expected_files", []):
        if expected_file and expected_file in prompt:
            raise RuntimeError(
                f"prompt for task {task['id']} includes expected file path {expected_file}"
            )


def parse_result_json(text: str) -> dict[str, Any]:
    try:
        value = json.loads(text)
    except json.JSONDecodeError:
        start = text.find("{")
        end = text.rfind("}")
        if start == -1 or end == -1 or end <= start:
            raise
        value = json.loads(text[start : end + 1])
    if not isinstance(value, dict):
        raise ValueError("Codex final response was not a JSON object")
    return value


def normalize_reported_file(raw: str, repo: Path, task_id: str) -> str:
    path = raw.strip().strip("`").lstrip("./")
    path = LINE_SUFFIX.sub("", path)
    candidate = Path(path)
    if candidate.is_absolute():
        try:
            path = candidate.resolve().relative_to(repo.resolve()).as_posix()
        except ValueError as exc:
            raise RuntimeError(
                f"task {task_id} reported file outside the measured repo: {raw}"
            ) from exc
    if path == ".." or path.startswith("../") or "/../" in path:
        raise RuntimeError(f"task {task_id} reported non-repo-relative file path: {raw}")
    if not (repo / path).is_file():
        raise RuntimeError(f"task {task_id} reported file that does not exist: {raw}")
    return path


def unique_existing_files(paths: Any, repo: Path, task_id: str) -> list[str]:
    if not isinstance(paths, list):
        raise RuntimeError(f"task {task_id} files_selected_or_read must be a list")
    seen: set[str] = set()
    files: list[str] = []
    for raw in paths:
        if not isinstance(raw, str):
            raise RuntimeError(f"task {task_id} reported a non-string file path: {raw!r}")
        path = normalize_reported_file(raw, repo, task_id)
        if not path or path in seen:
            continue
        seen.add(path)
        files.append(path)
    return files


def write_schema() -> None:
    write_json(SCHEMA_PATH, RESULT_SCHEMA)


def codex_task_command(prompt: str, model: str) -> list[str]:
    cmd = [
        codex_bin(),
        "--ask-for-approval",
        "never",
    ]
    for feature in DISABLED_CODEX_FEATURES:
        cmd.extend(["--disable", feature])
    cmd.extend(
        [
            "exec",
            "--ephemeral",
            "--json",
            "--ignore-user-config",
            "--ignore-rules",
            "-C",
            str(REQUESTS_REPO),
            "--sandbox",
            "read-only",
            "--output-schema",
            str(SCHEMA_PATH),
        ]
    )
    if model:
        cmd.extend(["--model", model])
    cmd.append(prompt)
    return cmd


def parse_codex_jsonl(stdout: str, stderr: str, task_id: str) -> dict[str, Any]:
    events: list[dict[str, Any]] = []
    non_json_stdout: list[str] = []
    for line in stdout.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        if stripped.startswith("{"):
            try:
                value = json.loads(stripped)
            except json.JSONDecodeError:
                non_json_stdout.append(line)
                continue
            if isinstance(value, dict):
                events.append(value)
            else:
                non_json_stdout.append(line)
        else:
            non_json_stdout.append(line)

    final_text = None
    usage = None
    for event in events:
        if event.get("type") == "item.completed":
            item = event.get("item")
            if isinstance(item, dict) and item.get("type") == "agent_message":
                final_text = item.get("text")
        if event.get("type") == "turn.completed":
            event_usage = event.get("usage")
            if isinstance(event_usage, dict):
                usage = event_usage

    if not isinstance(final_text, str) or not final_text.strip():
        raise RuntimeError(f"Codex JSONL transcript for {task_id} had no final agent message")
    if not isinstance(usage, dict):
        raise RuntimeError(f"Codex JSONL transcript for {task_id} had no turn.completed usage")

    parsed = parse_result_json(final_text)
    if parsed.get("task_id") != task_id:
        raise RuntimeError(f"Codex final response returned task_id {parsed.get('task_id')!r} for {task_id}")
    files = unique_existing_files(parsed.get("files_selected_or_read"), REQUESTS_REPO, task_id)
    context_tokens, token_fields = codex_context_tokens(usage, task_id)
    return {
        "events": events,
        "non_json_stdout": non_json_stdout,
        "stderr": stderr,
        "final_text": final_text,
        "parsed_result": parsed,
        "files_selected_or_read": files,
        "agent_native_context_tokens": context_tokens,
        "token_fields": token_fields,
    }


def codex_context_tokens(usage: dict[str, Any], task_id: str) -> tuple[int, dict[str, int]]:
    fields = {
        "input_tokens": int(usage.get("input_tokens") or 0),
        "cached_input_tokens": int(usage.get("cached_input_tokens") or 0),
        "output_tokens": int(usage.get("output_tokens") or 0),
        "reasoning_output_tokens": int(usage.get("reasoning_output_tokens") or 0),
    }
    context_tokens = fields["input_tokens"]
    if context_tokens <= 0:
        raise RuntimeError(f"Codex JSONL transcript for {task_id} has no input token usage")
    return context_tokens, fields


def run_codex_task(task: dict[str, Any], model: str) -> dict[str, Any]:
    prompt = prompt_for_task(task)
    cmd = codex_task_command(prompt, model)
    result = run(cmd, cwd=REQUESTS_REPO, check=False)
    if result.returncode != 0:
        raise CommandError(cmd, REQUESTS_REPO, result)
    parsed = parse_codex_jsonl(result.stdout, result.stderr, task["id"])
    if not parsed["files_selected_or_read"]:
        raise RuntimeError(f"Codex returned no files for task {task['id']}")
    return {
        "task_id": task["id"],
        "repo": task.get("repo"),
        "base_commit": task.get("base_commit"),
        "checked_out_commit": current_requests_head(),
        "prompt": prompt,
        "command": cmd,
        "parsed_result": parsed["parsed_result"],
        "files_selected_or_read": parsed["files_selected_or_read"],
        "agent_native_context_tokens": parsed["agent_native_context_tokens"],
        "token_accounting": {
            "source": "Codex CLI JSONL turn.completed usage.input_tokens",
            "context_tokens": parsed["agent_native_context_tokens"],
            "fields": parsed["token_fields"],
        },
        "raw_events": parsed["events"],
        "non_json_stdout": parsed["non_json_stdout"],
        "stderr": parsed["stderr"],
    }


def final_agent_message(events: list[dict[str, Any]], task_id: str) -> str:
    final_text = None
    for event in events:
        if event.get("type") == "item.completed":
            item = event.get("item")
            if isinstance(item, dict) and item.get("type") == "agent_message":
                final_text = item.get("text")
    if not isinstance(final_text, str) or not final_text.strip():
        raise RuntimeError(f"raw transcript for {task_id} has no final Codex agent message")
    return final_text


def final_usage(events: list[dict[str, Any]], task_id: str) -> dict[str, Any]:
    usage = None
    for event in events:
        if event.get("type") == "turn.completed":
            event_usage = event.get("usage")
            if isinstance(event_usage, dict):
                usage = event_usage
    if not isinstance(usage, dict):
        raise RuntimeError(f"raw transcript for {task_id} has no Codex turn.completed usage")
    return usage


def validate_task_result(
    task: dict[str, Any],
    value: dict[str, Any],
    path: Path,
    model: str,
) -> dict[str, Any]:
    task_id = task["id"]
    if value.get("task_id") != task_id:
        raise RuntimeError(f"raw transcript {path} task_id does not match {task_id}")
    if value.get("repo") != task.get("repo"):
        raise RuntimeError(f"raw transcript {path} repo does not match {task_id}")
    if value.get("base_commit") != task.get("base_commit"):
        raise RuntimeError(f"raw transcript {path} base_commit does not match {task_id}")
    if task.get("base_commit"):
        checked_out_commit = value.get("checked_out_commit")
        if checked_out_commit != task["base_commit"]:
            raise RuntimeError(f"raw transcript {path} checked_out_commit does not match {task_id}")
        current_head = current_requests_head()
        if current_head != task["base_commit"]:
            raise RuntimeError(
                f"Requests checkout is at {current_head}, expected {task['base_commit']} for {task_id}"
            )
    prompt = prompt_for_task(task)
    if value.get("prompt") != prompt:
        raise RuntimeError(f"raw transcript {path} prompt no longer matches the harness")
    expected_command = codex_task_command(prompt, model)
    if value.get("command") != expected_command:
        raise RuntimeError(f"raw transcript {path} command no longer matches the harness")
    raw_events = value.get("raw_events")
    if not isinstance(raw_events, list):
        raise RuntimeError(f"raw transcript {path} must include Codex JSONL raw_events")
    final_text = final_agent_message(raw_events, task_id)
    event_parsed = parse_result_json(final_text)
    parsed_result = value.get("parsed_result")
    if parsed_result != event_parsed:
        raise RuntimeError(f"raw transcript {path} parsed_result does not match raw events")
    if parsed_result.get("task_id") != task_id:
        raise RuntimeError(f"raw transcript {path} parsed_result task_id does not match {task_id}")
    files = unique_existing_files(value.get("files_selected_or_read"), REQUESTS_REPO, task_id)
    event_files = unique_existing_files(
        event_parsed.get("files_selected_or_read"),
        REQUESTS_REPO,
        task_id,
    )
    if files != event_files:
        raise RuntimeError(f"raw transcript {path} files do not match raw events")
    if not files:
        raise RuntimeError(f"raw transcript {path} has no files_selected_or_read")
    tokens = int(value.get("agent_native_context_tokens") or 0)
    if tokens <= 0:
        raise RuntimeError(f"raw transcript {path} has no positive agent_native_context_tokens")
    usage = final_usage(raw_events, task_id)
    event_tokens, token_fields = codex_context_tokens(usage, task_id)
    if tokens != event_tokens:
        raise RuntimeError(f"raw transcript {path} context tokens do not match raw events")
    token_accounting = value.get("token_accounting")
    if not isinstance(token_accounting, dict) or token_accounting.get("context_tokens") != tokens:
        raise RuntimeError(f"raw transcript {path} token_accounting does not match context tokens")
    if token_accounting.get("fields") != token_fields:
        raise RuntimeError(f"raw transcript {path} token_accounting fields do not match raw events")
    value["files_selected_or_read"] = files
    value["agent_native_context_tokens"] = tokens
    return value


def synthetic_task_result(task: dict[str, Any], selected_file: str, model: str) -> dict[str, Any]:
    prompt = prompt_for_task(task)
    final_text = json.dumps(
        {
            "task_id": task["id"],
            "files_selected_or_read": [selected_file],
            "rationale": "Synthetic harness self-test fixture.",
        },
        separators=(",", ":"),
    )
    raw_events = [
        {"type": "thread.started", "thread_id": "synthetic"},
        {"type": "turn.started"},
        {
            "type": "item.completed",
            "item": {"id": "item_0", "type": "agent_message", "text": final_text},
        },
        {
            "type": "turn.completed",
            "usage": {
                "input_tokens": 1234,
                "cached_input_tokens": 100,
                "output_tokens": 42,
                "reasoning_output_tokens": 0,
            },
        },
    ]
    parsed_result = parse_result_json(final_text)
    context_tokens, token_fields = codex_context_tokens(final_usage(raw_events, task["id"]), task["id"])
    return {
        "task_id": task["id"],
        "repo": task.get("repo"),
        "base_commit": task.get("base_commit"),
        "checked_out_commit": current_requests_head(),
        "prompt": prompt,
        "command": codex_task_command(prompt, model),
        "parsed_result": parsed_result,
        "files_selected_or_read": [selected_file],
        "agent_native_context_tokens": context_tokens,
        "token_accounting": {
            "source": "synthetic harness self-test",
            "context_tokens": context_tokens,
            "fields": token_fields,
        },
        "raw_events": raw_events,
        "non_json_stdout": [],
        "stderr": "",
    }


def expect_validation_failure(
    task: dict[str, Any],
    value: dict[str, Any],
    name: str,
    model: str,
) -> None:
    try:
        validate_task_result(task, value, Path(f"<synthetic-{name}>"), model)
    except RuntimeError:
        return
    raise RuntimeError(f"self-test expected validation failure for {name}")


def run_self_test(template: dict[str, Any], tasks: list[dict[str, Any]], model: str) -> None:
    if not tasks:
        raise RuntimeError("self-test requires at least one template task")
    task = tasks[0]
    starting_head = current_requests_head()
    try:
        checkout_task_repo(task)
        selected_files = task.get("callsieve_files")
        if not selected_files:
            raise RuntimeError(f"self-test task {task['id']} has no callsieve_files")
        selected_file = selected_files[0]
        if not isinstance(selected_file, str):
            raise RuntimeError(f"self-test task {task['id']} callsieve_files[0] is not a string")
        baseline = synthetic_task_result(task, selected_file, model)
        validate_task_result(task, copy.deepcopy(baseline), Path("<synthetic-valid>"), model)

        repo_mismatch = copy.deepcopy(baseline)
        repo_mismatch["repo"] = "other/repo"
        expect_validation_failure(task, repo_mismatch, "repo-mismatch", model)

        base_commit_mismatch = copy.deepcopy(baseline)
        base_commit_mismatch["base_commit"] = "0" * 40
        expect_validation_failure(task, base_commit_mismatch, "base-commit-mismatch", model)

        checkout_mismatch = copy.deepcopy(baseline)
        checkout_mismatch["checked_out_commit"] = "1" * 40
        expect_validation_failure(task, checkout_mismatch, "checkout-mismatch", model)

        prompt_mismatch = copy.deepcopy(baseline)
        prompt_mismatch["prompt"] = f"{prompt_mismatch['prompt']}\nDrift."
        expect_validation_failure(task, prompt_mismatch, "prompt-mismatch", model)

        command_mismatch = copy.deepcopy(baseline)
        command_mismatch["command"] = command_mismatch["command"] + ["--unexpected"]
        expect_validation_failure(task, command_mismatch, "command-mismatch", model)

        file_mismatch = copy.deepcopy(baseline)
        file_mismatch["files_selected_or_read"] = []
        expect_validation_failure(task, file_mismatch, "file-mismatch", model)

        token_mismatch = copy.deepcopy(baseline)
        token_mismatch["agent_native_context_tokens"] += 1
        expect_validation_failure(task, token_mismatch, "token-mismatch", model)

        accounting_mismatch = copy.deepcopy(baseline)
        accounting_mismatch["token_accounting"]["fields"]["input_tokens"] += 1
        expect_validation_failure(task, accounting_mismatch, "token-accounting-mismatch", model)
    finally:
        if current_requests_head() != starting_head:
            ensure_clean_requests_checkout()
            run(["git", "checkout", "-f", starting_head], cwd=REQUESTS_REPO)

    print(
        "Validated synthetic Codex raw-transcript provenance checks for "
        f"{task['id']} from {repo_relative(REQUESTS_REPO)}."
    )


def raw_task_path(task_id: str) -> Path:
    return RAW_DIR / f"{task_id}.json"


def load_or_run_codex_task(
    task: dict[str, Any],
    model: str,
    force: bool,
    finalize_only: bool,
) -> dict[str, Any]:
    path = raw_task_path(task["id"])
    if path.is_file() and not force:
        value = load_json(path)
        if not isinstance(value, dict):
            raise RuntimeError(f"raw transcript is not an object: {path}")
        return validate_task_result(task, value, path, model)
    if finalize_only:
        raise RuntimeError(f"missing raw transcript for finalize-only run: {path}")
    result = run_codex_task(task, model)
    write_json(path, result)
    return validate_task_result(task, result, path, model)


def write_measured_task_log(template: dict[str, Any], task_results: list[dict[str, Any]]) -> dict[str, Any]:
    by_id = {result["task_id"]: result for result in task_results}
    measured = copy.deepcopy(template)
    measured["status"] = "measured"
    measured["locally_measured"] = True
    measured["measurement_tool"] = "Codex CLI native search/read workflow"
    measured["measurement_note"] = (
        f"Filled by benchmarks/tools/codex-agent-native-requests.py --suite {CONFIG.suite} from Codex CLI JSONL transcripts."
    )
    for task in measured["tasks"]:
        result = by_id[task["id"]]
        task["agent_native_files"] = result["files_selected_or_read"]
        task["agent_native_context_tokens"] = result["agent_native_context_tokens"]
        task["recording_status"] = "measured"
    write_json(TASK_LOG_PATH, measured)
    return measured


def write_transcript_bundle(
    template: dict[str, Any],
    task_results: list[dict[str, Any]],
    version: str,
    model: str,
) -> None:
    bundle = {
        "tool": "Codex CLI",
        "tool_version": version,
        "model": model or "codex-cli-default",
        "suite": CONFIG.suite,
        "repo": repo_relative(REQUESTS_REPO),
        "pinned_base_commits": bool(template.get("pinned_base_commits")),
        "template": repo_relative(TEMPLATE_PATH),
        "task_log": repo_relative(TASK_LOG_PATH),
        "raw_task_transcripts": repo_relative(RAW_DIR),
        "result_schema": repo_relative(SCHEMA_PATH),
        "disabled_codex_features": DISABLED_CODEX_FEATURES,
        "token_accounting": "agent_native_context_tokens = input_tokens from each Codex CLI JSONL turn.completed usage event",
        "task_count": len(task_results),
        "tasks": task_results,
        "template_source_tasks_hash": template.get("source_tasks_hash"),
    }
    write_json(TRANSCRIPT_PATH, bundle)


def run_check_and_baseline(task_count: int) -> None:
    run(
        [
            "cargo",
            "run",
            "--",
            "agent-native-check",
            repo_relative(TASK_LOG_PATH),
            "--mode",
            "measured",
            "--source-artifact",
            repo_relative(TRANSCRIPT_PATH),
            "--out",
            repo_relative(CHECK_PATH),
        ]
    )
    run(
        [
            "cargo",
            "run",
            "--",
            "agent-native-baseline",
            repo_relative(TASK_LOG_PATH),
            "--id",
            CONFIG.baseline_id,
            "--tool",
            "Codex CLI native search/read workflow",
            "--k",
            "5",
            "--measurement-command",
            f"benchmarks/tools/codex-agent-native-requests.py --suite {CONFIG.suite}",
            "--source-artifact",
            repo_relative(TRANSCRIPT_PATH),
            "--measurement-note",
            f"{CONFIG.measurement_note} Task count: {task_count}.",
            "--out",
            repo_relative(BASELINE_PATH),
        ]
    )


def write_overlay_manifest(task_count: int, minimum_delta: float, minimum_ratio: float) -> None:
    manifest = load_json(BASE_PUBLIC_PROOF)
    baseline_entry = {
        "id": CONFIG.baseline_id,
        "tool": "Codex CLI native search/read workflow",
        "path": repo_relative(BASELINE_PATH),
        "command": f"benchmarks/tools/codex-agent-native-requests.py --suite {CONFIG.suite}",
        "required": True,
        "minimum_tasks": task_count,
        "minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k": minimum_delta,
        "minimum_agent_native_context_token_ratio_vs_callsieve": minimum_ratio,
    }
    baselines = [
        entry
        for entry in manifest.get("agent_native_search_baselines", [])
        if entry.get("id") != baseline_entry["id"]
    ]
    baselines.append(baseline_entry)
    manifest["agent_native_search_baselines"] = baselines

    check_entry = {
        "id": "agent-native-check",
        "path": repo_relative(CHECK_PATH),
        "kind": "json_artifact",
        "description": f"Measured Codex CLI native-search preflight check for public {CONFIG.label} tasks.",
        "required": True,
    }
    terminal_artifacts = [
        entry
        for entry in manifest.get("terminal_artifacts", [])
        if not (entry.get("id") == "agent-native-check" and entry.get("path") == check_entry["path"])
    ]
    terminal_artifacts.append(check_entry)
    manifest["terminal_artifacts"] = terminal_artifacts
    write_json(OVERLAY_MANIFEST_PATH, manifest)


def run_public_proof() -> dict[str, Any]:
    result = run(
        [
            "cargo",
            "run",
            "--",
            "public-proof-report",
            repo_relative(OVERLAY_MANIFEST_PATH),
        ]
    )
    proof = json.loads(result.stdout)
    write_json(PROOF_PATH, proof)
    return proof


def write_measurement_plan(
    template: dict[str, Any],
    tasks: list[dict[str, Any]],
    version: str,
    model: str,
) -> dict[str, Any]:
    write_schema()
    plan = {
        "tool": "Codex CLI",
        "tool_version": version,
        "model": model or "codex-cli-default",
        "suite": CONFIG.suite,
        "repo": repo_relative(REQUESTS_REPO),
        "template": repo_relative(TEMPLATE_PATH),
        "public_manifest": repo_relative(PUBLIC_MANIFEST),
        "raw_task_transcripts": repo_relative(RAW_DIR),
        "result_schema": repo_relative(SCHEMA_PATH),
        "disabled_codex_features": DISABLED_CODEX_FEATURES,
        "constraints": [
            "Do not use CallSieve, callsieve commands, callsieve MCP tools, or precomputed CallSieve context.",
            "Do not expose ground-truth files in Codex prompts.",
            "Do not edit files.",
            "Check out each task's pinned base_commit before running Codex.",
            "Run Codex CLI with plugins, memory, Chronicle, browser, apps, computer-use, multi-agent, goals, and hooks disabled.",
            "Run Codex CLI with --ignore-user-config, --ignore-rules, and --sandbox read-only.",
            "Record repo, base_commit, and checked_out_commit in every raw task transcript.",
            "Record files_selected_or_read from Codex's schema-constrained final JSON response.",
            "Record agent_native_context_tokens from Codex CLI JSONL turn.completed usage.input_tokens.",
        ],
        "post_run_artifacts": {
            "schema": repo_relative(SCHEMA_PATH),
            "protocol": repo_relative(PROTOCOL_PATH),
            "task_log": repo_relative(TASK_LOG_PATH),
            "transcript": repo_relative(TRANSCRIPT_PATH),
            "check": repo_relative(CHECK_PATH),
            "baseline": repo_relative(BASELINE_PATH),
            "overlay_manifest": repo_relative(OVERLAY_MANIFEST_PATH),
            "proof": repo_relative(PROOF_PATH),
        },
        "token_accounting": "agent_native_context_tokens = input_tokens from each Codex CLI JSONL turn.completed usage event",
        "template_source_tasks_hash": template.get("source_tasks_hash"),
        "template_index_fingerprint": template.get("index_fingerprint"),
        "template_retrieval_contract_fingerprint": template.get("retrieval_contract_fingerprint"),
        "task_count": len(tasks),
        "tasks": [
            {
                "task_id": task["id"],
                "repo": task.get("repo"),
                "base_commit": task.get("base_commit"),
                "task": task["task"],
                "prompt": prompt_for_task(task),
                "command": codex_task_command(prompt_for_task(task), model),
                "checkout_command": (
                    ["git", "checkout", "-f", task["base_commit"]]
                    if task.get("base_commit")
                    else None
                ),
                "raw_transcript": repo_relative(raw_task_path(task["id"])),
            }
            for task in tasks
        ],
    }
    write_json(PLAN_PATH, plan)
    return plan


def validate_measurement_plan(template: dict[str, Any], tasks: list[dict[str, Any]], model: str) -> None:
    write_schema()
    plan = load_json(PLAN_PATH)
    if plan.get("tool") != "Codex CLI":
        raise RuntimeError("measurement plan tool must be Codex CLI")
    if plan.get("suite") != CONFIG.suite:
        raise RuntimeError("measurement plan suite does not match the selected harness suite")
    if plan.get("repo") != repo_relative(REQUESTS_REPO):
        raise RuntimeError("measurement plan repo does not match the pinned checkout")
    if plan.get("template") != repo_relative(TEMPLATE_PATH):
        raise RuntimeError("measurement plan template path does not match")
    if plan.get("result_schema") != repo_relative(SCHEMA_PATH):
        raise RuntimeError("measurement plan result schema path does not match")
    if plan.get("disabled_codex_features") != DISABLED_CODEX_FEATURES:
        raise RuntimeError("measurement plan disabled Codex features drifted")
    if plan.get("task_count") != len(tasks):
        raise RuntimeError("measurement plan task_count does not match selected tasks")
    plan_tasks = plan.get("tasks")
    if not isinstance(plan_tasks, list):
        raise RuntimeError("measurement plan tasks must be a list")
    if len(plan_tasks) != len(tasks):
        raise RuntimeError("measurement plan tasks length does not match selected tasks")

    for task, planned in zip(tasks, plan_tasks, strict=True):
        if planned.get("task_id") != task["id"]:
            raise RuntimeError(f"measurement plan task order mismatch for {task['id']}")
        if planned.get("repo") != task.get("repo"):
            raise RuntimeError(f"measurement plan repo drifted for {task['id']}")
        if planned.get("base_commit") != task.get("base_commit"):
            raise RuntimeError(f"measurement plan base_commit drifted for {task['id']}")
        prompt = prompt_for_task(task)
        if planned.get("prompt") != prompt:
            raise RuntimeError(f"measurement plan prompt drifted for {task['id']}")
        expected_command = codex_task_command(prompt, model)
        if planned.get("command") != expected_command:
            raise RuntimeError(f"measurement plan command drifted for {task['id']}")
        expected_checkout = (
            ["git", "checkout", "-f", task["base_commit"]]
            if task.get("base_commit")
            else None
        )
        if planned.get("checkout_command") != expected_checkout:
            raise RuntimeError(f"measurement plan checkout command drifted for {task['id']}")
        if planned.get("raw_transcript") != repo_relative(raw_task_path(task["id"])):
            raise RuntimeError(f"measurement plan raw transcript path drifted for {task['id']}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--suite",
        choices=sorted(SUITES),
        default="requests",
        help="Public task suite to measure.",
    )
    parser.add_argument(
        "--model",
        default="",
        help="Optional Codex model name. Empty uses the Codex CLI default.",
    )
    parser.add_argument(
        "--limit-tasks",
        type=int,
        default=0,
        help="Plan/debug only: select the first N tasks for --plan-only or --validate-plan.",
    )
    parser.add_argument(
        "--minimum-delta",
        type=float,
        default=0.0,
        help="Minimum CallSieve minus Codex first-correct-file@5 delta for public proof.",
    )
    parser.add_argument(
        "--minimum-ratio",
        type=float,
        default=2.0,
        help="Minimum Codex context-token ratio versus CallSieve packet tokens for public proof.",
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Rerun Codex tasks even when per-task raw transcripts already exist.",
    )
    parser.add_argument(
        "--finalize-only",
        action="store_true",
        help="Do not call Codex. Rebuild task log, check, baseline, manifest, and proof from existing raw task transcripts.",
    )
    parser.add_argument(
        "--plan-only",
        action="store_true",
        help="Write the exact measurement plan and prompts, then exit before auth checks or Codex calls.",
    )
    parser.add_argument(
        "--validate-plan",
        action="store_true",
        help="Validate the saved measurement plan against the current template and harness command construction.",
    )
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="Run synthetic raw-transcript provenance checks without authenticating or calling Codex.",
    )
    args = parser.parse_args()
    configure_suite(args.suite)

    with HarnessLock():
        run_harness(args)


def run_harness(args: argparse.Namespace) -> None:
    version = codex_version()
    prepare_protocol()
    template = prepare_template()
    write_schema()
    tasks = template["tasks"]
    full_task_count = len(tasks)
    if args.limit_tasks:
        tasks = tasks[: args.limit_tasks]
    if not tasks:
        raise SystemExit("No tasks selected for measurement.")
    if args.plan_only:
        write_measurement_plan(template, tasks, version, args.model)
        print(f"Wrote {repo_relative(PLAN_PATH)}")
        return
    if args.validate_plan:
        validate_measurement_plan(template, tasks, args.model)
        print(f"Validated {repo_relative(PLAN_PATH)}")
        return
    if args.self_test:
        run_self_test(template, tasks, args.model)
        return
    require_codex_login()
    if args.limit_tasks:
        raise SystemExit(
            "--limit-tasks is for plan validation only. Refusing to write measured public proof "
            f"artifacts for {len(tasks)} of {full_task_count} {CONFIG.label} tasks."
        )

    task_results = []
    for index, task in enumerate(tasks, start=1):
        checkout_task_repo(task)
        path = raw_task_path(task["id"])
        verb = "Rerunning" if args.force or not path.is_file() else "Reusing"
        if args.finalize_only:
            verb = "Loading"
        print(f"[{index}/{len(tasks)}] {verb} {task['id']} with Codex CLI", file=sys.stderr)
        task_results.append(
            load_or_run_codex_task(
                task,
                args.model,
                args.force,
                args.finalize_only,
            )
        )

    measured_template = copy.deepcopy(template)
    measured_template["tasks"] = tasks
    write_measured_task_log(measured_template, task_results)
    write_transcript_bundle(measured_template, task_results, version, args.model)
    run_check_and_baseline(len(tasks))
    write_overlay_manifest(len(tasks), args.minimum_delta, args.minimum_ratio)
    proof = run_public_proof()

    guardrail_status = proof["agent_native_search_guardrail"]["status"]
    print(f"Wrote {repo_relative(SCHEMA_PATH)}")
    print(f"Wrote {repo_relative(TASK_LOG_PATH)}")
    print(f"Wrote {repo_relative(TRANSCRIPT_PATH)}")
    print(f"Wrote {repo_relative(CHECK_PATH)}")
    print(f"Wrote {repo_relative(BASELINE_PATH)}")
    print(f"Wrote {repo_relative(OVERLAY_MANIFEST_PATH)}")
    print(f"Wrote {repo_relative(PROOF_PATH)}")
    print(f"public-proof-report status: {proof['status']}")
    print(f"agent_native_search_guardrail status: {guardrail_status}")
    if proof["status"] != "pass" or guardrail_status == "not_measured":
        raise SystemExit(1)


if __name__ == "__main__":
    try:
        main()
    except (CommandError, RuntimeError) as err:
        raise SystemExit(str(err)) from err
