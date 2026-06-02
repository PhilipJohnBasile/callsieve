# CallSieve Local Agent Policy

Use CallSieve as the first codebase-discovery step for coding tasks in this repo.

## Required Workflow

1. Call `callsieve_context` with the repository path and task before broad grep, `rg`, repository-wide search, or repeated file reads.
2. Read the returned `read_first` files and snippets first.
3. Use `callsieve_symbol` for named symbols when the task is symbol-centric.
4. Check `callsieve_status` if index freshness or LSP enrichment matters.
5. Grep only when the context packet is insufficient, and keep the grep narrow.

## Current App Shape

CallSieve is a local Rust CLI and MCP server. It indexes code locally, returns compact agent context, can enrich references with installed local language servers, keeps indexes fresh with watch or daemon commands, and records observed-session evidence for proof reports.

Do not send repository code to remote services as part of CallSieve workflows. The local JSON index lives under `.callsieve/index.json`.

## Proof Discipline

Controlled replay and observed sessions are separate. `codex-session` is useful for local setup and replay checks, but broad claims require observed-session traces collected with `session-start`, `session-event`, and `session-finish`, then validated by `proof-report` or `enterprise-proof-report`.
