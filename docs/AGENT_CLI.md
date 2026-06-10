# AI CLI Runbook

This document is for AI coding agents, AI CLIs, wrappers, and schedulers that run CallSieve automatically.

CallSieve is the local codebase context layer. Its job is to select the right files, symbols, snippets, tests, and blast-radius hints before an agent spends tokens on broad search or repeated file reads.

CallSieve retrieval uses zero AI model tokens because it ranks against a local deterministic index. The context packet it returns still consumes agent context tokens when read.

Humans installing CallSieve or adding it to Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo Code, the deprecated Roo alias, or another stdio MCP tool should start with [INSTALL.md](INSTALL.md).

## Core Contract

Before broad repository search, repeated file reads, or speculative exploration, run CallSieve.

Required first command for a coding task:

```bash
callsieve agent-context <repo> "<task>"
```

Use `--format markdown` when the agent should read a compact text packet instead of parsing JSON. Markdown includes the packet token estimate, symbol-scoped local expansion commands, and capped graph hints. Use `focus` or a richer profile for call-path detail. JSON remains the default for tooling.
The default packet is `--profile skim --token-budget 1200` and intentionally omits snippets. Ask for more only when needed:

```bash
callsieve focus <repo> --file <file> [--symbol <symbol>] [--line <line>] [--references]
callsieve related <repo> --file <file>
callsieve tests <repo> --file <file>
```

If CallSieve is being run from this Rust source checkout instead of an installed binary, use:

```bash
cargo run -- agent-context <repo> "<task>"
```

Optional signals are explicit. Use them only when the repo has the matching local artifacts or the task calls for them:

```bash
callsieve index <repo> --embeddings
callsieve agent-context <repo> "<task>" --embeddings --git-boost
callsieve agent-context <repo> "<task>" --error <stacktrace.log>
```

`--embeddings` requires a binary built with `--features embed`; otherwise the CLI exits with a clear feature-gate error. The default path remains lexical and deterministic. `--git-boost` uses recent local git activity as a ranking nudge. `--error` parses stack traces and promotes files named by resolved frames. Record these flags in traces or benchmark notes when they are part of a measured run.

MCP equivalent:

```json
{
  "tool": "callsieve_context",
  "arguments": {
    "path": "<repo>",
    "task": "<task>",
    "limit": 8,
    "snippets_per_file": 0
  }
}
```

Do not start with `rg`, `grep`, `find`, `Get-Content`, `cat`, `sed`, `nl`, `read_file`, or directory-wide listing unless CallSieve is unavailable or the task is not about codebase discovery.

## Interface Choice

Use the simplest available interface:

- Lifecycle hooks or plugins available for Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, or Cline: rely on injected CallSieve context, local expansion commands, and hook blocking before broad search.
- MCP/rules/template setup available for Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, or Zoo: rely on generated setup plus strict shims, not lifecycle hooks.
- Hook launcher available: start the agent through `.callsieve/agent-launch.ps1` or `.callsieve/agent-launch.sh` so repo-local shims and daemon startup apply only to that process.
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

`callsieve agent-context`, `callsieve context`, `callsieve begin`, `callsieve guard`, `callsieve codex-session`, `callsieve grep`, and MCP `callsieve_context` rebuild a missing or stale local index before returning context. Direct CLI agents should still run `callsieve index` explicitly when non-context commands such as `query`, `symbol`, `focus`, `related`, or `tests` fail because the index is missing.

## Per-Task Workflow

1. Run `callsieve agent-context <repo> "<task>"`.
2. Parse `context.retrieval_cost`.
3. Parse `context.stats.local` to see how many files, symbols, and references CallSieve searched locally before the packet was produced. Default skim packets use `f`, `sy`, and `r`.
4. Parse `context.sel` for the top selected file, capped `next`, top reasons, and capped local score signals.
5. Parse `context.read_first[]`.
6. Read the returned compact file, symbol, reason, test, risk, and `g` hints first.
7. Read full files only for the returned `read_first[].f` paths that are needed for the edit. Normal/full packets may use `file`.
8. Use `instruction.x` targets to expand the top selected file with `focus` and any emitted related or test follow-ups. Default `agent-context` JSON uses zero-based read-first indexes: `x.o: 0` means `context.read_first[0]`, `x.n: 1` means focus `context.read_first[1]`, and compact `x.r: 1` plus `x.t: 1` means run local `related` and `tests` for the top file. Legacy or wider packets may use `x.top: 0` or `x.next: [1]`. Older/full target objects may still use `f` for file, `sy` for symbol, `l` for line, `rel`, and `tests`. Docs and config top hits may omit code-only `r` and `t`. Symbol focus returns the selected code unit as a bounded snippet up to 120 lines by default, with `truncated` and `omitted_lines` only when that cap is hit, plus compact `calls`, `called_by`, and `related_tests` hints for the selected symbol. Non-call `references` are opt-in with `--references` because they can be noisy. When present, use the capped `x.n` or `x.next` target to focus the next ranked file before grep.
9. Use `callsieve symbol` or `callsieve query` for narrower follow-up lookups.
10. Use broad grep only if the context packet and local expansion commands are insufficient.
11. If broad grep is needed, keep it focused and explain why the CallSieve packet was insufficient.

Useful follow-up commands:

```bash
callsieve symbol <repo> <symbol_name> --limit 20
callsieve query <repo> "<question>" --limit 10
callsieve context <repo> "<task>" --limit 8 --snippets-per-file 1 --why-debug
callsieve agent-context <repo> "<task>" --format markdown
callsieve agent-context <repo> "<task>" --embeddings --why-debug
```

Use `context.sel` for compact default ranking explainability. Default skim caps `sel.next` to one next-ranked file without enabling verbose scoring. Skim encodes `sel.top` as `[index, why]`, `sel.sig` entries as code strings, and `sel.next` entries as `[index, why]` when the file is in `context.read_first`; array order carries the ranking signal. If a selected file is not in `read_first`, CallSieve falls back to `[path, score, why]`. Use `--why-debug` only when diagnosing deeper ranking behavior. It adds per-file scoring detail and costs more context.

`--token-budget` applies to the full serialized `agent-context` response. When local task-memory hints or optional local-expansion commands would push the packet over budget, CallSieve compacts memory first, trims `instruction.x.n` or `instruction.x.next`, trims lower-priority expansion commands such as related and test lookups, removes optional `g` and `cp` hints, then drops lower-ranked `read_first` files only if needed. The default `skim` packet selects up to five read-first files, omits the long retrieval-cost note, per-file `rank`, per-file score fields, per-file `language`, default git hints, common `function` symbol kinds, default memory hint arrays, non-action `instruction.a`, default JSON `context.root`, and empty symbol/test/count fields, uses `f`, `sy`, optional non-duplicate `w`, and `i` instead of `file`, `symbols`, `why`, and `impact`, uses `b` and `t` instead of `budget` and `tokens`, uses symbol arrays such as `["createSession",12]` instead of symbol objects, uses trailing compact kind codes such as `s` for `struct` and `c` for `constant` only when the kind is not the implicit `function`, uses compact reason codes such as `sym:` for exact symbols, `sy:` for symbol-name terms, `kw:` for generic query terms, `ct:` for content terms, `pt:` for path terms, and `test:` for test companions, drops duplicate generic `kw:` when a matching `sy:` reason is already present, and keeps `retrieval_model_tokens = 0`; `--git-boost` keeps compact `git` hints in skim, while normal/full packets may include the explanatory note, compact memory hint arrays, and fuller file metadata. Pass `--limit` when a task needs a wider first packet.

## JSON Fields To Use

`agent-context` returns:

```json
{
  "instruction": {
    "x": {
      "o": 0,
      "n": 1,
      "r": 1,
      "t": 1
    }
  },
  "context": {
    "retrieval_cost": {
      "retrieval_model_tokens": 0
    },
    "sel": {
      "top": [0, "sym:createSession"],
      "sig": ["sym"],
      "next": [[1, "test:related test proximity"]]
    },
    "read_first": [
      {
        "f": "<top-file>",
        "sy": [["<symbol>", 10]],
        "i": ["m", "<test-file>", 1, 1],
        "g": {
          "u": ["<called-or-imported-file>"],
          "d": ["<caller-or-referencing-file>"]
        }
      }
    ],
    "stats": {
      "b": 1200,
      "t": 900,
      "local": {
        "f": 100,
        "sy": 1000,
        "r": 5000
      }
    },
    "timing": {},
    "warnings": []
  }
}
```

Default skim keeps task-memory matching local and omits a hit-only `memory` envelope because it does not tell the agent what to read or run next. Normal/full packets can expose `memory.hit`, `memory.f`, and `memory.sy` as hints from previous local tasks, capped at two files and three symbols. They can reduce repeated search, but they do not replace `context.read_first` and they are not proof evidence. If the user asks for a cold run, execute:

Default skim packets omit the ordinary task echo because the agent already supplied it. When CallSieve recovers an anaphoric follow-up such as `fix 1-5`, `context.task` is included so the recovered retrieval target is visible.

```bash
callsieve memory-clear <repo>
```

For each `context.read_first[]` item, prioritize:

- `f`: repo-relative file path to inspect. Normal/full packets may use `file`.
- array order: ordering signal. Default skim omits per-file `rank` and score fields; normal/full packets may use `score`.
- `sy`: likely relevant symbol name and line guidance, omitted in skim when empty. Skim caps this list at one per file. Each skim symbol is `[name,line]` or the same array with a trailing compact non-`function` kind code. Common codes include `s` for `struct`, `c` for `constant`, `cl` for `class`, `m` for `method`, `mod` for `module`, `mac` for `macro`, and `cmp` for `component`. Normal/full packets may use `symbols`, `name`, `kind`, `line`, and `lines`.
- `snippets`: compact code excerpts and line ranges.
- `w`: optional short explanation for top-file reasons not already carried by `context.sel.top`. Default skim omits `w` when it would only repeat the selected reason; use `context.sel.top` and `context.sel.next` first, then normal/full packets for per-file reasons across the whole list. Default skim uses compact reason codes: `sym:` exact symbol, `sy:` symbol-name terms, `kw:` generic query terms, `ct:` content terms, `p:` path match, `pt:` path terms, `doc:` symbol docs, `im:` imports, `ref:` references, `call:` calls, and `test:` test companion. Normal/full packets may use `why`.
- `context.sel.top`, `context.sel.sig`, `context.sel.next`: compact skim ranking explanation arrays. `top` is `[index, why]`; each `sig` entry is a signal code string; each `next` entry is `[index, why]`. Resolve the index through `context.read_first`; read-first array order carries ranking. If a selected file is not in `read_first`, CallSieve falls back to `[path, score, why]`. Common signal codes mirror reason codes where possible: `sym` exact symbol, `sy` symbol-name cluster, `kw` keyword overlap, `ct` content overlap, `p` path or filename, `pt` path terms, `test` test proximity, `comp` competitive-positioning doc, `cmd` command surface, and `git` local git signal. Normal/full packets may use `selection_summary`.
- `imports`, `referenced_by`, `calls`, `called_by`: graph hints in normal/full packets.
- `related_tests`: tests likely affected by the change in normal/full packets.
- `i`: compact skim impact array. Positions are `[risk, tests, upstream, downstream]`; `tests` is a string for one external test path, a number for one zero-based `context.read_first` index, or an array for multiple test paths and read-first indexes. Trailing zero counts are omitted. Skim risk codes are `l`, `m`, and `h` for low, medium, and high. Normal/full packets may use `impact`.
- `g.u` and `g.d`: one local non-test upstream/downstream dependency preview for the top file in skim packets. Use `focus` or `related` for graph detail on lower-ranked files.
- `cp.c` and `cp.by`: capped cross-file caller/callee hints when explicitly included by a richer path. Default skim omits them so agents use local `focus` before spending first-packet tokens on call paths. Edge keys are `f` for file, `fr` for source symbol, `t` for target symbol, and `l` for line.
- `blast_radius.tests`, `blast_radius.imports`, `blast_radius.referenced_by`: richer impact hints in normal/full packets.
- `git`: recent local git activity when indexed, useful as context rather than proof by itself. In skim packets, `lm` is last modified unix time, `c90` is commits over 90 days, and `a90` is authors over 90 days.
- `stats.b` and `stats.t`: skim packet budget and returned context token estimate. Normal/full packets may use `token_budget`, `budget`, `estimated_tokens`, or `tokens`.
- `stats.local`: indexed files, symbols, and references searched locally with zero retrieval-model tokens. Skim packets use compact keys: `f`, `sy`, and `r`. Normal/full packets may use `files`, `symbols`, and `refs`.

For `callsieve focus`, prefer passing `--symbol` plus `--line` from `context.read_first[].sy[0]` before reading the whole file. In skim, use `sy[0][0]` for symbol and `sy[0][1]` for line, or resolve `instruction.x.o`, legacy `instruction.x.top`, `instruction.x.n`, and `instruction.x.next` as indexes into `context.read_first`. Normal/full packets may use `sy[].n`, `sy[].l`, or the first value in `sy[].ls`. File-level focus without `--symbol` stays compact; symbol focus returns the selected local code unit unless it exceeds the cap and includes capped local caller/callee/test hints. Add `--references` only when non-call reference edges are needed. `--line` disambiguates same-name symbols in one file.

If `warnings` reports that CallSieve rebuilt a missing or stale index before context, the returned packet already used the refreshed local index. If another command reports stale index entries, rerun:

```bash
callsieve index <repo> --lsp
```

If `warnings` reports a missing or stale embeddings cache, rerun:

```bash
callsieve index <repo> --embeddings
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

Use `callsieve grep <repo> "<query>"` when an agent wants a grep-like fallback but should still receive compact CallSieve context first. Its nested context is the default `skim` packet with the standard token budget; raw `rg` only runs when `--run-rg` is explicitly set.

## Self-Healing For AI CLIs

Safe local repair commands:

```bash
callsieve index <repo> --lsp
callsieve codex-hooks install <repo> --strict --force
callsieve codex-hooks doctor <repo> --strict --smoke
callsieve codex-hooks trust-ack <repo>
callsieve claude-hooks install <repo> --strict --force
callsieve claude-hooks doctor <repo> --strict
callsieve copilot-hooks install <repo> --strict --force
callsieve copilot-hooks doctor <repo> --strict
callsieve opencode-hooks install <repo> --strict --force
callsieve opencode-hooks doctor <repo> --strict
callsieve antigravity-hooks install <repo> --strict --force
callsieve antigravity-hooks doctor <repo> --strict
callsieve cline-hooks install <repo> --strict --force
callsieve cline-hooks doctor <repo> --strict
callsieve hook install <repo> --client generic --strict --force --lsp
callsieve hook doctor <repo>
callsieve doctor <repo> --client generic --fix --strict
callsieve mcp-config <repo> --format json
callsieve mcp-config <repo> --format toml
callsieve mcp-registry-manifest --out server.json
callsieve proof-rehearsal --preflight
callsieve proof-rehearsal --fix --resume
```

For Codex, `codex-hooks doctor --strict --smoke` validates the `slim` hook profile with local handler smoke tests. Add `--fix` when stale hook state or trace files need to be archived under `.callsieve/codex-hooks/archive/`. After a human reviews project hooks in Codex with `/hooks`, `codex-hooks trust-ack <repo>` records a local marker tied to the current hook file hash.

`proof-rehearsal --fix` is intentionally limited. It can create ignored local evidence directories, rebuild local indexes, and regenerate missing controlled replay traces. It does not clone repos, install tools, delete evidence, record observed sessions, or run claim proof.

Do not mutate global PATH, global shell profiles, editor global settings, or cloud configuration unless the user explicitly asks.

## Session Learning, Receipts, And Team Transfer

Observed sessions teach retrieval and produce auditable evidence:

```bash
# After hook-served sessions, files the agent actually read become
# confirmed task-memory associations automatically. Recall them:
callsieve agent-context <repo> "<task>" --memory-boost

# Tamper-evident summary of the most recent observed session
# (packets, packet tokens, reads, broad searches, edit impacts):
callsieve receipt <repo> --latest
callsieve receipts <repo>          # per-repo rollup across sessions

# Ship a warm index and learned associations to teammates or CI:
callsieve index-export <repo> --out team-index.json
callsieve memory-export <repo> --out team-memory.json
callsieve index-import <repo> --from team-index.json
callsieve memory-import <repo> --from team-memory.json
```

Editing an indexed file through a hook-capable client (Claude Code, Copilot, OpenCode, Antigravity, Cline) returns an impact note — callers, related tests, blast-radius risk — as additional context on the edit event; follow it with `callsieve focus` before broad reads.

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

`retrieval_cost`:

- Scope: local CallSieve retrieval only.
- `retrieval_model_tokens` is `0` because ranking runs against `.callsieve/index.json`.
- The returned context packet, later full-file reads, and the rest of the agent session still consume agent context tokens.

`context_payload_reduction`:

- Platform-neutral proxy.
- Works across Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo Code, the deprecated Roo alias, generic stdio MCP tools, and local agents.
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
Use `callsieve agent-context <repo> "<task>"`.
Read the returned `context.read_first` files, symbols, and line ranges first.
Use `instruction.x` for `focus` and any emitted `rel` or `tests` detail when the CallSieve packet is insufficient, and keep any broad grep focused.
Treat `retrieval_cost.retrieval_model_tokens = 0` as retrieval-only.
When reporting savings, call `context_payload_reduction` an estimated context payload reduction, not observed whole-session token savings.
Run `proof-report` only after the claim-counted manifest passes `pilot-qa`.
Do not send proprietary code to remote services for CallSieve operations.
```

## Failure Handling

If a non-context command fails because the index is missing:

```bash
callsieve index <repo> --lsp
callsieve agent-context <repo> "<task>"
```

If an MCP client needs config:

```bash
callsieve mcp-config <repo> --format json
callsieve mcp-config <repo> --format toml
callsieve mcp-registry-manifest --out server.json
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
