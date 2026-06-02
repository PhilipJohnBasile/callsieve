# CallSieve Product Brief

## One-liner

CallSieve is the local codebase filter for AI coding agents.

## Problem

Agentic coding tools waste massive context and cost rediscovering codebases through repeated grep, ripgrep, file reads, directory scans, and duplicate exploration.

This makes coding agents:
- slower
- more expensive
- less reliable
- worse on large repos
- worse with smaller local models

## Insight

The best way to improve coding agents is not only bigger context. It is better context selection.

CallSieve is sparse attention for the codebase before the prompt exists.

Sparse attention helps a model spend less compute on irrelevant prompt tokens. CallSieve helps an agent avoid putting irrelevant repo tokens into the prompt in the first place.

The distinction matters:

- CallSieve solves finding information.
- Sparse attention solves reasoning over information already found.
- If the relevant function is not in the prompt, sparse attention cannot find it.

Agents should not ask:

```bash
rg "UserService"
```

They should ask:

```text
show_symbol(UserService)
find_callers(UserService)
find_tests(UserService)
explain_module(auth)
```

## Retrieval Model

CallSieve should use a coarse-to-fine retrieval pipeline:

```text
User question
  -> repo index
  -> top-k files, symbols, tests, and call neighborhoods
  -> relevant snippets
  -> compact context for the coding agent
```

The first implementation should use deterministic ranking:

- exact symbol matches
- path and filename matches
- import proximity
- test proximity
- keyword overlap

Later versions can add semantic reranking, embeddings, richer call graphs, and durable cross-repo agent memory.

## Current Product

CallSieve is a local Rust CLI and MCP-compatible context server. It indexes a repository into `.callsieve/index.json`, ranks relevant files and symbols for a task, returns compact read-first context packets, and audits whether agents used CallSieve before broad grep or repeated reads.

The current app includes:

- repository walking with ignore-rule support
- language detection for TypeScript, JavaScript, Python, Rust, docs, and common config files
- symbol, import, reference, call, and related-test extraction
- optional local LSP enrichment when language servers are already installed
- deterministic query and `agent-context` ranking with optional scoring debug output
- local task-memory hints for repeated task families
- MCP tools for context, symbols, stats, status, trace checks, and benchmark estimates
- portable MCP config output for generic AI CLIs
- watcher and daemon paths for index freshness
- first-class adoption automation with `bootstrap`, `doctor`, `begin`, agent setup, Codex bootstrap, editor hooks, and opt-in grep shims
- benchmark, retrieval-eval, perf-report, observed trace, pilot, evidence-pack, proof-report, and enterprise-proof-report workflows

## Current CLI Entry Points

```bash
callsieve index <path> [--lsp]
callsieve agent-context <path> "<task>"
callsieve demo <path> [--task "<task>"]
callsieve memory-clear <path>
callsieve symbol <path> <symbol_name>
callsieve status <path>
callsieve mcp
callsieve mcp-config <path> [--format json|toml]
callsieve bootstrap <path> --client <client> [--strict] [--force] [--lsp]
callsieve doctor <path> --client <client> [--fix] [--strict]
callsieve begin <path> "<task>" --client <client> [--trace-out <trace.json>]
callsieve eval-retrieval <manifest.json>
callsieve perf-report <path>
callsieve benchmark-report <manifest.json>
callsieve proof-report <manifest.json>
callsieve enterprise-proof-report <manifest.json>
```

## Proof Program

CallSieve now has three evidence tiers:

- rehearsal evidence from deterministic retrieval fixtures, benchmark reports, platform-neutral context payload reduction, perf reports, and controlled replay
- supplemental evidence from Ollama or local-model runs that can test prompts and expected files without counting as Codex proof
- claim-counted evidence from real paired Codex sessions with transcript token accounting, transcript-backed files read, strict trace policy, and `pilot-qa` passing before `proof-report`

`context_payload_reduction` is the cross-agent proxy metric. It estimates the repo context payload CallSieve avoids versus deterministic grep/read replay, and can be compared across Codex, Claude, Gemini, Kimi, Cursor, Cline, Roo, and local agents. It is not observed whole-session token savings.

The rehearsal layer should be self-healing for local-safe issues: rebuild indexes, regenerate controlled traces, resume passed steps, retry transient process failures, and emit scheduler-friendly JSON. Claim-counted proof should stay gated on real transcript evidence.

The product claim stays gated. Broad developer-session language should wait until `enterprise-proof-report` returns `pass`.

## Near-Term Product Direction

The next product work should harden the same local-first path:

- richer call graph and ownership hints
- broader language coverage
- better task-category and repo-scale reporting
- tighter retrieval fixtures and p95 latency gates before broad claims
- editor and agent integrations that make `agent-context` the default first step
- optional semantic reranking only after deterministic retrieval proves the core loop

## Core Differentiator

Most tools help agents generate code.

CallSieve helps agents stop wasting tokens before they generate code.

The product should be judged by one question:

Can it make a coding agent read the right 5 files instead of grepping through 50?
