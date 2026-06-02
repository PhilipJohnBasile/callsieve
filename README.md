# CallSieve

CallSieve is the local codebase filter for AI coding agents.

It indexes a repository and returns compact, structured context so agents can spend fewer tokens on blind grep, file discovery, repeated reads, and rediscovering project structure.

## Keystone Thesis

> "Almost all context windows for developer sessions, truly massive percentages, are filled up by grepping. If people came up with a solution for less grepping in projects, that would save a lot of tokens and would probably be worth money."
>
> - Microsoft engineer

CallSieve exists to turn that observation into infrastructure.

## Product Promise

Stop paying AI agents to grep your repo.

CallSieve is not another coding agent. It is the context and retrieval layer underneath coding agents.

## MVP Commands

```bash
callsieve index <path> [--lsp]
callsieve symbols <path>
callsieve symbol <path> <symbol_name>
callsieve query <path> "<question>"
callsieve context <path> "<task>" [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve agent-context <path> "<task>" [--limit <n>] [--snippets-per-file <n>]
callsieve benchmark <path> "<task>" [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve benchmark-suite <path> <tasks.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve trace-summary <trace.json>
callsieve session-start <path> "<task>" --client codex --model <name> --trace <trace.json> [--expected-file <path>] [--critical-file <path>]
callsieve session-event <trace.json> --command <cmd> [--files-read <path>...] [--tokens <n>] [--phase baseline|callsieve]
callsieve session-finish <trace.json> --out <summary.json>
callsieve trace-replay <path> <tasks.json> <trace.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve trace-check <trace.json> [--strict]
callsieve benchmark-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve benchmark-doctor <manifest.json>
callsieve pilot-init <manifest.json> [--sessions <n>]
callsieve pilot-task add <manifest.json> <repo> "<task>" [--id <id>] [--expected-file <path>] [--critical-file <path>] [--external]
callsieve pilot-run <manifest.json> --task-id <id> --mode baseline|callsieve --command <cmd> [--files-read <path>...] --tokens <n>
callsieve pilot-qa <manifest.json>
callsieve pilot-finalize <manifest.json> --out <proof.json>
callsieve pilot-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve proof-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve pilot-doctor <manifest.json>
callsieve evidence-pack <manifest.json> [--anonymize]
callsieve policy-check <trace.json> [--strict]
callsieve mcp
callsieve status <path>
callsieve daemon <path> [--background] [--foreground] [--once] [--lsp]
callsieve daemon-status <path>
callsieve daemon-stop <path>
callsieve watch <path> [--debounce-ms <n>] [--foreground] [--lsp]
callsieve agent-setup <path> --client <codex|claude|cursor|cline|roo|generic> [--force]
callsieve setup-agent <codex|claude|cursor|cline|roo|generic> <path> [--force]
callsieve codex-bootstrap <path> --model <name> [--force]
callsieve editor-hook <path> --editor <vscode|cursor|generic> [--force]
callsieve guard <path> "<task>" [--trace-out <trace.json>]
callsieve codex-session <path> "<task>" --trace-out <trace.json> [--model <name>] [--expected-file <path>]
callsieve enforce <path> --client <codex|claude|cursor|cline|roo|generic> [--trace <trace.json>] [--strict] [--require-shim]
callsieve shim install <path> [--force]
callsieve shim doctor <path>
callsieve shim uninstall <path>
callsieve grep <path> "<query>" [--run-rg]
callsieve stats <path>
```

Example:

```bash
cargo run -- index .
cargo run -- index . --lsp
cargo run -- query . "where is auth handled?"
cargo run -- context . "change login token expiry behavior"
cargo run -- agent-context . "change login token expiry behavior"
cargo run -- benchmark . "change login token expiry behavior"
cargo run -- benchmark-suite . benchmarks/tasks.json
cargo run -- benchmark-suite . benchmarks/callsieve-real-repo.json
cargo run -- trace-summary benchmarks/session-trace.example.json
cargo run -- session-start . "change login token expiry behavior" --client codex --model gpt-5-codex --trace .callsieve/observed-session.json
cargo run -- session-event .callsieve/observed-session.json --command "callsieve agent-context . \"change login token expiry behavior\"" --files-read src/auth/session.ts --tokens 3000 --phase callsieve
cargo run -- session-finish .callsieve/observed-session.json --out .callsieve/observed-summary.json
cargo run -- trace-replay . benchmarks/callsieve-real-repo.json benchmarks/session-trace.local.json --limit 20
cargo run -- trace-check benchmarks/session-trace.example.json --strict
cargo run -- benchmark-report benchmarks/report-manifest.example.json
cargo run -- benchmark-doctor benchmarks/report-manifest.example.json
cargo run -- pilot-init benchmarks/evidence/pilot.local.json --sessions 50
cargo run -- pilot-task add benchmarks/evidence/pilot.local.json . "change login token expiry behavior" --id auth-expiry --expected-file src/auth/session.ts --critical-file src/auth/session.ts
cargo run -- pilot-run benchmarks/evidence/pilot.local.json --task-id auth-expiry --mode baseline --command "rg login token expiry" --files-read src/auth/session.ts --tokens 12000
cargo run -- pilot-run benchmarks/evidence/pilot.local.json --task-id auth-expiry --mode callsieve --command "callsieve agent-context . \"change login token expiry behavior\"" --files-read src/auth/session.ts --tokens 3000
cargo run -- pilot-qa benchmarks/evidence/pilot.local.json
cargo run -- pilot-finalize benchmarks/evidence/pilot.local.json --out benchmarks/evidence/proof.local.json
cargo run -- pilot-report benchmarks/pilot-manifest.example.json
cargo run -- proof-report benchmarks/pilot-manifest.example.json
cargo run -- pilot-doctor benchmarks/pilot-manifest.example.json
cargo run -- evidence-pack benchmarks/pilot-manifest.example.json --anonymize
cargo run -- policy-check benchmarks/session-trace.example.json --strict
cargo run -- mcp
cargo run -- status .
cargo run -- daemon . --once
cargo run -- daemon-status .
cargo run -- watch .
cargo run -- watch . --lsp
cargo run -- agent-setup . --client codex
cargo run -- codex-bootstrap . --model gpt-5-codex --force
cargo run -- editor-hook . --editor cursor --force
cargo run -- guard . "change login token expiry behavior" --trace-out .callsieve/session-trace.json
cargo run -- codex-session . "change login token expiry behavior" --trace-out .callsieve/codex-session.json --model gpt-5-codex
cargo run -- enforce . --client codex --trace .callsieve/session-trace.json --strict
cargo run -- shim install . --force
cargo run -- shim doctor .
cargo run -- grep . "change login token expiry behavior"
```

## What The MVP Does

- walks a repository while respecting common ignore rules
- detects TypeScript, JavaScript, Python, and Rust source files plus agent-relevant docs and config files
- extracts practical symbols with tree-sitter-backed parsing and deterministic fallbacks
- extracts imports, references, and calls
- can enrich references with local Language Server Protocol servers when `--lsp` is enabled
- indexes bounded content terms for Markdown, JSON, TOML, YAML, and text without returning full files
- stores a local JSON index at `.callsieve/index.json`
- returns compact JSON for agent consumption
- ranks matches with deterministic, explainable scoring
- builds compact read-first context packets for coding tasks
- boosts package manifests for dependency and setup tasks
- boosts context with import, caller, and callee proximity
- provides an `agent-context` wrapper agents can call before grep
- exposes a minimal MCP stdio server so agents can call CallSieve before grep
- estimates context-packet token savings versus a naive grep/read loop
- records real observed Codex/ChatGPT session events and summarizes baseline versus CallSieve-assisted phases
- keeps controlled replay evidence separate from observed-session evidence
- aggregates benchmark evidence across multiple local repositories
- produces pilot and top-level proof reports that combine benchmark, observed trace, controlled replay, policy, freshness, bootstrap, daemon, and LSP evidence
- produces anonymized evidence packs for external pilot aggregation
- provides CI-friendly strict policy checks for context-first sessions
- validates evidence manifests before reports
- reports index freshness and keeps indexes fresh with a local watcher or daemon state loop
- generates client-specific agent rules that require CallSieve before broad grep
- guards context-first sessions and can write trace stubs for policy audits
- starts controlled Codex/ChatGPT context-first replay traces with model tags
- bootstraps project-local Codex launchers, config, rules, and grep shims without global PATH/profile mutation
- generates project-local editor hooks for VS Code, Cursor, and generic editors
- audits agent setup, traces, index freshness, and optional shim state with `enforce`
- installs opt-in local `rg`/`grep` shims for PATH-level interception
- wraps grep workflows so CallSieve context is returned before optional `rg`

## Example Query Output

```json
{
  "query": "where is auth handled?",
  "root": ".",
  "matches": [
    {
      "rank": 1,
      "score": 90,
      "file": "src/auth/session.ts",
      "language": "typescript",
      "symbol": {
        "name": "createSession",
        "kind": "function",
        "lines": [12, 48],
        "visibility": "exported",
        "signature": "export function createSession(...)"
      },
      "why": ["keyword overlap: auth, session"]
    }
  ],
  "stats": {
    "searched_files": 182,
    "matched_files": 7,
    "matched_symbols": 12
  }
}
```

## Example Context Output

```json
{
  "task": "change login token expiry behavior",
  "root": ".",
  "read_first": [
    {
      "rank": 1,
      "score": 140,
      "file": "src/auth/session.ts",
      "language": "typescript",
      "symbols": [
        {
          "name": "createSession",
          "kind": "function",
          "lines": [12, 48],
          "visibility": "exported",
          "signature": "export function createSession(...)"
        }
      ],
      "snippets": [
        {
          "lines": [12, 30],
          "text": "export function createSession(...) { ... }"
        }
      ],
      "imports": ["src/auth/token.ts"],
      "referenced_by": ["src/auth/session.test.ts"],
      "blast_radius": {
        "imports": ["src/auth/token.ts"],
        "referenced_by": ["src/auth/session.test.ts"],
        "tests": ["src/auth/session.test.ts"],
        "calls": ["src/auth/token.ts"],
        "risk": "medium"
      },
      "calls": [
        {
          "file": "src/auth/session.ts",
          "symbol": "createSession",
          "target": "tokenFor",
          "target_file": "src/auth/token.ts",
          "kind": "call",
          "line": 13
        }
      ],
      "related_tests": [
        {
          "file": "src/auth/session.test.ts",
          "symbols": ["createSession returns token-backed session"]
        }
      ],
      "why": [
        "exact symbol match: createSession",
        "keyword overlap: auth, session",
        "references matched file: src/auth/token.ts"
      ]
    }
  ],
  "stats": {
    "candidate_matches": 30,
    "selected_files": 5,
    "selected_symbols": 8,
    "related_tests": 2
  }
}
```

## Example Benchmark Output

```json
{
  "task": "change login token expiry behavior",
  "estimator": "local deterministic token estimate",
  "baseline": {
    "strategy": "naive grep term scan plus full matched-file reads",
    "grep_terms": ["login", "token", "expiry"],
    "grep_commands": 3,
    "matched_files": 18,
    "estimated_total_tokens": 24000
  },
  "callsieve": {
    "strategy": "callsieve context packet",
    "selected_files": 6,
    "estimated_packet_tokens": 4200
  },
  "savings": {
    "avoided_grep_commands": 2,
    "avoided_file_reads": 12,
    "estimated_token_savings": 19800,
    "estimated_token_reduction_percent": 82.5
  }
}
```

## Example Benchmark Suite

```json
{
  "tasks": [
    {
      "id": "auth-token-expiry",
      "task": "change login token expiry behavior",
      "expected_files": ["src/auth/session.ts", "src/auth/token.ts"],
      "observed": {
        "baseline": { "grep_commands": 12, "file_reads": 18, "tokens": 42000 },
        "callsieve": { "grep_commands": 1, "file_reads": 6, "tokens": 9000 }
      }
    }
  ]
}
```

`benchmark-suite` reports expected-file recall, aggregate estimated token savings, and optional observed session savings when real agent trace numbers are supplied.

`trace-replay` generates deterministic baseline versus CallSieve trace JSON from a suite. It is tagged with `metadata.collection = "controlled_replay"` and is useful before real observed session evidence exists.

Use `session-start`, `session-event`, and `session-finish` for real Codex/ChatGPT observations. These traces are tagged with `metadata.collection = "observed_session"` and keep ordered events with command classification, files read, optional token counts, and phase (`baseline` or `callsieve`).

See [docs/BENCHMARKS.md](docs/BENCHMARKS.md) for the real-repo benchmark pack, session trace format, replay traces, and miss analysis fields.

## Example Trace Summary

```json
{
  "sessions": 3,
  "baseline_tokens": 84000,
  "callsieve_tokens": 27000,
  "token_savings": 57000,
  "token_reduction_percent": 67.85714285714286,
  "avoided_grep_commands": 18,
  "avoided_file_reads": 31,
  "files_still_missed": 1
}
```

## Example Benchmark Report Manifest

```json
{
  "repos": [
    {
      "label": "callsieve",
      "path": ".",
      "suite_path": "benchmarks/callsieve-real-repo.json",
      "trace_path": "benchmarks/session-trace.example.json"
    }
  ]
}
```

`benchmark-report` does not clone repositories or use the network. Every repo path, suite path, and trace path must already exist locally.

Use `benchmark-doctor` before a report to catch missing repos, missing indexes, bad suite JSON, and bad trace JSON.

## Example Pilot Report Manifest

```json
{
  "thresholds": {
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": 50.0,
    "minimum_observed_sessions": 1,
    "minimum_observed_token_reduction_percent": 50.0,
    "minimum_external_repos": 0,
    "maximum_controlled_replay_ratio": 0.25,
    "maximum_trace_violations": 0,
    "maximum_critical_misses": 0,
    "require_fresh_index": true,
    "require_lsp_where_available": false,
    "require_codex_bootstrap": false
  },
  "repos": [
    {
      "label": "callsieve",
      "path": ".",
      "languages": ["typescript", "javascript", "python", "rust"],
      "suite_paths": ["benchmarks/callsieve-real-repo.json"],
      "trace_paths": ["benchmarks/session-trace.example.json"],
      "policy_trace_paths": ["benchmarks/session-trace.example.json"]
    }
  ]
}
```

`pilot-report` is the pilot-proof artifact: it combines multi-repo benchmark recall, estimated token savings, observed trace savings, controlled replay counts, strict before-grep policy checks, index freshness, daemon state, Codex bootstrap coverage, and LSP coverage.

`proof-report` is the top-level claim artifact. It exposes observed sessions, controlled replay sessions, external repo coverage, observed token reduction, controlled replay ratio, freshness, daemon, bootstrap, and LSP status in one JSON object. Controlled replay is never counted as observed evidence.

Use `evidence-pack` when you need a shareable aggregate for external pilots:

```bash
cargo run -- evidence-pack benchmarks/pilot-manifest.example.json --anonymize
```

With `--anonymize`, repo paths, labels, suite paths, and trace paths are redacted while aggregate metrics remain intact.

## Agent Enforcement

Use `agent-setup` to install local MCP config plus a short CallSieve-first policy file for Codex, Claude, Cursor, Cline, Roo, or generic MCP clients:

```bash
cargo run -- agent-setup . --client codex --force
```

For coding tasks, the policy is: call `callsieve_context` before broad grep, `rg`, repository-wide search, or repeated file reads. Read `read_first` files first; grep only if the context packet is insufficient.

Use `guard` to start a context-first task and write a trace stub, then use strict `trace-check` to audit actual sessions:

```bash
cargo run -- guard . "change login token expiry behavior" --trace-out .callsieve/session-trace.json
cargo run -- trace-check .callsieve/session-trace.json --strict
cargo run -- policy-check .callsieve/session-trace.json --strict
cargo run -- enforce . --client codex --trace .callsieve/session-trace.json --strict
```

`policy-check` exits nonzero when a trace violates the context-first rule, so it can be used in CI. `enforce` checks generated agent files, index freshness, optional trace policy, and shim state. Missing shims are a warning unless `--require-shim` is set.

For Codex/ChatGPT-only pilots, use `codex-session` instead of a generic guard. It writes a trace with `client: codex-chatgpt`, a model label, a deterministic grep/read baseline, and a CallSieve-first assisted side:

```bash
cargo run -- codex-session . "change login token expiry behavior" --trace-out .callsieve/codex-session.json --model gpt-5-codex
cargo run -- trace-summary .callsieve/codex-session.json
cargo run -- enforce . --client codex --trace .callsieve/codex-session.json --strict
```

Run the same task with different `--model` labels when comparing available Codex/ChatGPT models. CallSieve records and audits the sessions you run; it does not invoke hidden ChatGPT models itself.

`codex-session` is controlled replay evidence. For real observed sessions, use:

```bash
cargo run -- session-start . "change login token expiry behavior" --client codex --model gpt-5-codex --trace .callsieve/observed-session.json
cargo run -- session-event .callsieve/observed-session.json --command "callsieve agent-context . \"change login token expiry behavior\"" --files-read src/auth/session.ts --tokens 3000 --phase callsieve
cargo run -- session-finish .callsieve/observed-session.json --out .callsieve/observed-summary.json
```

Use `codex-bootstrap` for Codex-first project setup without mutating global shell profiles or user PATH:

```bash
cargo run -- codex-bootstrap . --model gpt-5-codex --force
```

It writes `.codex/config.toml`, `.codex/CALLSIEVE.md`, `.callsieve/bin` shims, and `.callsieve/codex-launch.ps1` / `.callsieve/codex-launch.sh`. The launchers start `callsieve daemon --background --lsp`, prepend `.callsieve/bin` only for that launched process, and print the first required `callsieve agent-context` command.

This repo includes `benchmarks/codex-chatgpt-manifest.local.json` as the local Codex pilot manifest.

For hard opt-in grep interception, install local wrappers and prepend `.callsieve/bin` to the agent shell PATH:

```bash
cargo run -- shim install . --force
cargo run -- shim doctor .
```

The shim wrappers call `callsieve grep` before passing through to the real `rg` or `grep` command captured at install time.

## Fresh Indexes

`status` reports index freshness, schema version, watch status, watcher mode, index age, stale/changed/removed files, LSP server availability, and whether the saved index was actually LSP-enriched. `watch` refreshes the index once by default, or continuously when run with `--foreground`:

```bash
cargo run -- status .
cargo run -- watch .
cargo run -- watch . --lsp
cargo run -- watch . --foreground
```

The V1 watcher is a portable polling refresh path with no extra daemon dependency. It keeps the on-disk JSON index current while preserving the local-first model.

Use `daemon` for a stateful local refresh loop:

```bash
cargo run -- daemon . --once
cargo run -- daemon . --background --lsp
cargo run -- daemon . --foreground --lsp
cargo run -- daemon-status .
cargo run -- daemon-stop .
```

The daemon writes `.callsieve/daemon.json` with PID, `started_at`, `last_indexed_at`, `last_error`, and `index_generation`. `status` includes the saved daemon state. Background start is available through `callsieve daemon <path> --background`; foreground or `--once` is easier to inspect during pilots.

## LSP Enrichment

The default index is fast and deterministic. Add `--lsp` when you want CallSieve to ask installed local language servers for higher-confidence reference edges:

```bash
cargo run -- index . --lsp
```

CallSieve does not install servers, clone repositories, or use the network. It detects these local commands when the matching language is indexed:

- TypeScript/JavaScript: `typescript-language-server --stdio`
- Python: `pyright-langserver --stdio`
- Rust: `rust-analyzer`

If a server is missing or fails, CallSieve keeps the tree-sitter and heuristic graph and reports per-language availability plus failure reasons in `status`. LSP-derived edges use sources such as `"lsp_reference"`, `"lsp_definition"`, `"lsp_implementation"`, and `"lsp_type_definition"` with `"confidence": 1.0`; tree-sitter edges use `0.8`, and heuristic edges use `0.5`.

## MCP Integration

`callsieve mcp` runs a stdio JSON-RPC server with these tools:

- `callsieve_context`: preferred first tool for codebase discovery; build the compact read-first packet for a coding task before grep
- `callsieve_symbol`: find indexed symbols with import and reference hints
- `callsieve_stats`: inspect index coverage
- `callsieve_status`: inspect freshness, watch, schema, and LSP enrichment state
- `callsieve_trace_check`: audit whether a session grepped before CallSieve
- `callsieve_benchmark`: estimate grep/read-loop token savings

The MCP server requires an existing `.callsieve/index.json`; it does not rebuild or mutate the index.

See [docs/MCP.md](docs/MCP.md) for Codex, Claude Code, Claude Desktop, Cursor, Cline, and Roo setup examples.

## Local-First Guarantees

- no cloud services
- no API keys
- no proprietary code leaves the machine
- no SaaS app, auth system, web dashboard, or vector DB in the MVP

## Retrieval Model

CallSieve is sparse attention for the codebase before the prompt exists.

```text
User question
  -> repo index
  -> top-k files, symbols, tests, and import neighbors
  -> compact snippets
  -> agent context
```

The MVP uses deterministic ranking first:

- exact symbol match
- exact path or filename match
- exported symbol substring match
- local symbol substring match
- keyword overlap
- likely related tests
- direct import neighbors where available

Embeddings, Git history, editor-specific extensions, and long-term memory are later phases.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
