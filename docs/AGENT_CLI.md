# AI CLI Runbook

This document is for AI coding agents, AI CLIs, wrappers, and schedulers that run CallSieve automatically.

CallSieve is the local codebase context layer. Its job is to select the right files, symbols, snippets, tests, and blast-radius hints before an agent spends tokens on broad search or repeated file reads.

Humans installing CallSieve or adding it to Codex, Claude, Cursor, Cline, Roo, Gemini CLI, Kimi CLI, or another tool should start with [INSTALL.md](INSTALL.md).

## Core Contract

Before broad repository search, repeated file reads, or speculative exploration, run CallSieve.

Required first command for a coding task:

```bash
callsieve agent-context <repo> "<task>" --limit 8 --snippets-per-file 2
```

If CallSieve is being run from this Rust source checkout instead of an installed binary, use:

```bash
cargo run -- agent-context <repo> "<task>" --limit 8 --snippets-per-file 2
```

MCP equivalent:

```json
{
  "tool": "callsieve_context",
  "arguments": {
    "path": "<repo>",
    "task": "<task>",
    "limit": 8,
    "snippets_per_file": 2
  }
}
```

Do not start with `rg`, `grep`, `find`, `Get-Content`, `cat`, `sed`, `nl`, `read_file`, or directory-wide listing unless CallSieve is unavailable or the task is not about codebase discovery.

## Interface Choice

Use the simplest available interface:

- MCP client available: call `callsieve_context` first.
- Plain AI CLI available: run `callsieve agent-context`.
- Working inside the CallSieve repo before install: run `cargo run -- agent-context`.
- Evidence automation: use the Rust CLI commands in this document, not shell scripts.

The CLI emits JSON on success. Treat nonzero exit codes as failures and parse the JSON error if one is returned.

## Index Setup

The direct CLI expects a local index to exist. For a new repo, run:

```bash
callsieve index <repo> --lsp
```

Use `--lsp` when local language servers may already be installed. CallSieve stays local and falls back to tree-sitter and heuristic edges when LSP is unavailable.

Check state with:

```bash
callsieve status <repo>
```

For MCP, `callsieve_context` can rebuild a missing or stale index before returning context. Direct CLI agents should index explicitly when `agent-context`, `context`, `query`, or `symbol` fails because the index is missing.

## Per-Task Workflow

1. Run `callsieve agent-context <repo> "<task>"`.
2. Parse `instruction.action`. It should be `read_first_before_grep`.
3. Parse `context.read_first[]`.
4. Read the returned snippets first.
5. Read full files only for the returned `read_first[].file` paths that are needed for the edit.
6. Use `callsieve symbol` or `callsieve query` for narrower follow-up lookups.
7. Use broad grep only if the context packet is insufficient.
8. If broad grep is needed, keep it focused and explain why the CallSieve packet was insufficient.

Useful follow-up commands:

```bash
callsieve symbol <repo> <symbol_name> --limit 20
callsieve query <repo> "<question>" --limit 10
callsieve context <repo> "<task>" --limit 8 --snippets-per-file 2 --why-debug
```

Use `--why-debug` only when diagnosing ranking behavior. It adds scoring detail and costs more context.

## JSON Fields To Use

`agent-context` returns:

```json
{
  "instruction": {
    "action": "read_first_before_grep",
    "guidance": "Read these files first; grep only if insufficient.",
    "grep_policy": "grep_only_if_context_is_insufficient"
  },
  "memory": {
    "cache_hit": false,
    "policy": "local_project_memory_only; use as hints, not proof",
    "similar_tasks": [],
    "recommended_files": [],
    "recommended_symbols": []
  },
  "context": {
    "task": "...",
    "root": "...",
    "read_first": [],
    "stats": {},
    "timing": {},
    "warnings": []
  }
}
```

Use `memory.recommended_files` and `memory.recommended_symbols` as hints from previous local tasks. They can reduce repeated search, but they do not replace `context.read_first` and they are not proof evidence. If the user asks for a cold run, execute:

```bash
callsieve memory-clear <repo>
```

For each `context.read_first[]` item, prioritize:

- `file`: repo-relative file path to inspect.
- `rank` and `score`: ordering signal.
- `symbols`: likely relevant symbols and line ranges.
- `snippets`: compact code excerpts and line ranges.
- `why`: short explanation for selection.
- `imports`, `referenced_by`, `calls`, `called_by`: graph hints.
- `related_tests`: tests likely affected by the change.
- `blast_radius.risk`: rough change-risk signal.
- `blast_radius.tests`, `blast_radius.imports`, `blast_radius.referenced_by`: impact hints.

If `warnings` contains stale index entries, consider rerunning:

```bash
callsieve index <repo> --lsp
```

## Grep Policy

Allowed before CallSieve:

- Checking the current working directory.
- Reading explicit user-provided files.
- Running `git status`.
- Running build or test commands the user requested.

Not allowed before CallSieve for codebase discovery:

- Broad `rg`, `grep`, or `find`.
- Recursive file listing to discover implementation files.
- Reading many files to locate likely code.
- Opening common files repeatedly without a specific reason.

Allowed after CallSieve:

- Focused grep inside returned files or directories.
- Follow-up grep for a missing symbol, route, config key, or test name.
- Full file reads for selected `read_first` files.

## Self-Healing For AI CLIs

Safe local repair commands:

```bash
callsieve index <repo> --lsp
callsieve doctor <repo> --client generic --fix --strict
callsieve mcp-config <repo> --format json
callsieve mcp-config <repo> --format toml
callsieve proof-rehearsal --preflight
callsieve proof-rehearsal --fix --resume
```

`proof-rehearsal --fix` is intentionally limited. It can create ignored local evidence directories, rebuild local indexes, and regenerate missing controlled replay traces. It does not clone repos, install tools, delete evidence, record observed sessions, or run claim proof.

Do not mutate global PATH, global shell profiles, editor global settings, or cloud configuration unless the user explicitly asks.

## Evidence Commands

For deterministic local shakedown:

```bash
callsieve proof-rehearsal --preflight
callsieve proof-rehearsal --fix --resume
```

A passing rehearsal should include:

- `status: "pass"`
- `preflight.failures: 0`
- `command_matrix.report_limit: 24`
- `command_matrix.includes_proof_report: false`
- `claim_proof_included: false`
- `context_payload_reduction`

For external benchmark evidence:

```bash
callsieve benchmark-doctor benchmarks/external-github-manifest.example.json
callsieve benchmark-report benchmarks/external-github-manifest.example.json --limit 24
```

Expected current external fixture gate:

- `summary.expected_files: 28`
- `summary.expected_files_found: 28`
- `summary.missed_expected_files: 0`

For observed Codex collection setup:

```bash
callsieve setup-observed-codex-oss-50
callsieve pilot-qa benchmarks/evidence/observed-codex-oss-50.local.json
```

Before real sessions are recorded, `pilot-qa` must fail with `observed_sessions: 0`. That is correct and honest.

To record a real observed phase:

```bash
callsieve record-codex-observed-session --task-id <task-id> --mode baseline --command "<baseline command>" --tokens <transcript-context-tokens> --files-read <file>
callsieve record-codex-observed-session --task-id <task-id> --mode callsieve --command "callsieve agent-context <repo> \"<task>\"" --tokens <transcript-context-tokens> --files-read <file>
```

Rules for observed sessions:

- Do not estimate `tokens`.
- Use only real transcript token counts.
- Include at least one actual file read.
- Record baseline and CallSieve phases separately.
- Run `pilot-qa` after recording.
- Run `proof-report` only after `pilot-qa` passes.

## Metrics Contract

CallSieve reports two different kinds of savings evidence.

`context_payload_reduction`:

- Platform-neutral proxy.
- Works across Codex, Claude, Gemini, Kimi, Cursor, Cline, Roo, and local agents.
- Estimates the repo context payload avoided versus deterministic grep/read replay.
- Good wording: "estimated context payload reduction."
- Bad wording: "observed session token savings."

Observed token reduction:

- Platform-specific transcript evidence.
- Requires paired real sessions.
- Requires auditable transcript token counts.
- Can support stronger claims after `pilot-qa` and `proof-report` pass.

Never mix these metrics.

## Minimal AI System Instruction

Use this block in an AI CLI or agent policy:

```text
For every codebase-discovery task, run CallSieve before broad search or repeated file reads.
Use `callsieve agent-context <repo> "<task>" --limit 8 --snippets-per-file 2`.
Read the returned `context.read_first` snippets and files first.
Use broad grep only when the CallSieve packet is insufficient, and keep it focused.
When reporting savings, call `context_payload_reduction` an estimated context payload reduction, not observed whole-session token savings.
Run `proof-report` only after the claim-counted manifest passes `pilot-qa`.
Do not send proprietary code to remote services for CallSieve operations.
```

## Failure Handling

If `agent-context` fails because the index is missing:

```bash
callsieve index <repo> --lsp
callsieve agent-context <repo> "<task>" --limit 8 --snippets-per-file 2
```

If an MCP client needs config:

```bash
callsieve mcp-config <repo> --format json
callsieve mcp-config <repo> --format toml
```

If `proof-rehearsal --preflight` fails:

```bash
callsieve proof-rehearsal --fix --resume
```

If `benchmark-doctor` fails, fix only the reported missing local prerequisites. Do not clone, install, or delete unless the user explicitly approves.

If `pilot-qa` fails with `observed_sessions: 0`, do not try to force proof. Record real paired sessions first.

## Local-First Boundaries

CallSieve should not require:

- cloud services
- API keys
- vector databases
- SaaS auth
- sending proprietary code outside the machine

Agents may use CallSieve with any model or CLI surface, but CallSieve itself remains local and deterministic by default.
