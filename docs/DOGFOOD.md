# Dogfood Runbook

Use this runbook when CallSieve is tested by CallSieve, Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo Code, the deprecated Roo alias, or another stdio MCP tool.

## Goal

Reduce repeated repo discovery by making the first codebase-discovery command:

```bash
callsieve agent-context <repo> "<task>"
```

For MCP clients, the equivalent first tool is `callsieve_context`.
For MCP Registry discoverability checks, generate the local descriptor with `callsieve mcp-registry-manifest --out server.json`; this does not publish or contact the network.

## Cold Start

```bash
callsieve demo <repo> --task "find where CLI commands are defined"
callsieve mcp-config <repo> --format json
callsieve agent-context <repo> "find where CLI commands are defined"
```

`demo` verifies the full local loop: index, read-first files, and platform-neutral `context_payload_reduction`.

`agent-context` and MCP `callsieve_context` include `retrieval_cost.retrieval_model_tokens = 0`. Treat that as local retrieval cost only. The returned packet and any follow-up file reads still consume agent context tokens.

## Repeated Task Families

`agent-context` writes `.callsieve/task-memory.json`, an ignored local hint cache. The cache stores prior task terms, read-first files, selected symbols, and related tests.

Use the memory object as hints:

```json
{
  "memory": {
    "cache_hit": true,
    "recommended_files": ["src/cli.rs"],
    "recommended_symbols": ["Command"]
  }
}
```

Rules:

- Treat `context.read_first` as the primary source.
- Treat `memory.recommended_files` as a shortcut for repeated task families.
- Do not count task memory as proof evidence.
- Clear memory before cold-run comparisons:

```bash
callsieve memory-clear <repo>
```

## Measuring Less Grep

For local platform-neutral measurement:

```bash
callsieve benchmark <repo> "<task>"
callsieve proof-rehearsal --preflight
callsieve proof-rehearsal --fix --resume
```

Use the phrase `estimated context payload reduction` for `context_payload_reduction`. This applies across AI platforms because it measures the prompt payload avoided versus deterministic grep/read replay.

Use the phrase `zero AI model tokens for retrieval` only for the local ranking step. Do not describe whole agent sessions as zero-token unless the transcript evidence actually says so.

For observed whole-session token savings, record real paired sessions only:

```bash
callsieve record-codex-observed-session --task-id <task-id> --mode baseline --command "<baseline command>" --tokens <transcript-context-tokens> --files-read <file>
callsieve record-codex-observed-session --task-id <task-id> --mode callsieve --command "callsieve agent-context <repo> \"<task>\"" --tokens <transcript-context-tokens> --files-read <file>
```

Do not estimate transcript token counts. Do not use controlled replay as observed-session evidence.

## Trace Policy

Audit order of operations:

```bash
callsieve trace-check <trace.json> --strict
```

Important fields:

- `grep_before_context`: sessions that searched before CallSieve.
- `grep_after_context`: sessions that searched only after CallSieve context.
- `context_first_compliant`: true when no policy violations were found.

Focused grep after CallSieve is allowed when the packet is insufficient. Broad grep before CallSieve is a policy failure.

## Agent Instruction

Use this as the compact AI policy:

```text
For codebase discovery, call CallSieve before broad search.
Use MCP `callsieve_context` when available. Otherwise run `callsieve agent-context <repo> "<task>"`.
Read `context.read_first` first. Use `memory.recommended_files` only as local hints.
Remember `retrieval_cost.retrieval_model_tokens = 0` applies to retrieval only; returned context still counts when read.
Use broad grep only if the packet is insufficient, then keep it focused and explain why.
Report `context_payload_reduction` only as estimated context payload reduction.
Run `proof-report` only after the claim-counted manifest passes `pilot-qa`.
```
