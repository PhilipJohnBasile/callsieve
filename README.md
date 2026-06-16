# CallSieve

[![CI](https://github.com/PhilipJohnBasile/callsieve/actions/workflows/ci.yml/badge.svg)](https://github.com/PhilipJohnBasile/callsieve/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/PhilipJohnBasile/callsieve)](https://github.com/PhilipJohnBasile/callsieve/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Homebrew](https://img.shields.io/badge/brew-philipjohnbasile%2Fcallsieve-orange)](https://github.com/PhilipJohnBasile/homebrew-callsieve)

CallSieve is the local codebase filter for AI coding agents.

It indexes a repository and returns compact, structured context so agents can spend fewer tokens on blind grep, file discovery, repeated reads, and rediscovering project structure.

Retrieval itself spends zero AI model tokens: CallSieve ranks against a local index before the prompt exists. The returned context packet still consumes agent context tokens when read, so every packet is compact by default.

## Product Promise

Stop paying AI agents to grep your repo.

CallSieve is not another coding agent. It is the context and retrieval layer underneath coding agents.

## Competitive Posture

CallSieve is built to stay slim while making the strongest practical token-saving case:

- Slimmest architecture: Rust CLI, local `.callsieve/index.json`, deterministic ranking, no cloud service, no API key, no vector database, and no web dashboard.
- Best agent-agnostic setup story: Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo Code, Roo, and generic stdio MCP clients.
- Strongest proof posture: `benchmark`, `eval-retrieval`, `trace-check`, `trace-replay`, `pilot-*`, `proof-report`, `enterprise-proof-report`, and `evidence-pack`.
- Strongest token-saving positioning: read these files first, grep only if needed. CallSieve retrieval costs zero AI model tokens; the compact packet is the only token-bearing artifact agents need to read first.

## How CallSieve Compares

As of June 2026, CallSieve is best compared as retrieval infrastructure, not as a replacement coding assistant.

| Category | Examples | How CallSieve compares |
| --- | --- | --- |
| AI coding IDEs | Cursor, Windsurf, Continue | These bundle chat, edits, autocomplete, and context indexing. CallSieve is narrower: local deterministic retrieval, MCP, hooks, and compact context packets. It is stronger for portability and auditability, but weaker on polished IDE UX and semantic search. |
| Agent CLIs | Claude Code, Codex CLI, Aider | Claude Code and Codex are full agents that read, edit, run commands, and search files. CallSieve sits in front of them as a context-first tool. Aider is the closest technical peer because its repo map gives LLMs compact codebase structure, but Aider is still an editing agent. |
| Enterprise code intelligence | Sourcegraph Cody and Sourcegraph Search | Sourcegraph is broader: enterprise search, code graph, IDE and web experiences, and multi-repo context. CallSieve is slimmer: local CLI, local JSON index, no cloud, no API key, no vector DB, and no dashboard. |
| Semantic RAG and code indexers | Embedding or vector-backed codebase tools | These can be better for vague semantic discovery. CallSieve is more deterministic, explainable, local-first, and proof-oriented, but it can miss queries that need semantic matching beyond symbols, paths, imports, tests, and keywords. |

The wedge is agent-agnostic setup plus local proof: CallSieve can make Codex, Claude Code, GitHub Copilot, Cursor, Windsurf, Continue, or any MCP client read the right files first, then audit whether that happened. It wins when a team wants a vendor-neutral context layer across many agents. It loses when the buyer only wants one integrated AI editor.

## Open Source And Commercial Model

CallSieve's core local engine is open source under the [MIT License](LICENSE).

The public repo includes the local CLI, MCP server, repository indexer, deterministic retrieval, agent-context workflow, benchmark harness, proof reports, docs, and tests. The commercial motion is not selling access to the code. It is selling outcomes around it: paid pilots, local installation, agent integration, evidence collection, retrieval tuning, private workflow support, and enterprise proof reporting.

For proposed commercial packages and placeholder pricing, see [commercial/PRICING.md](commercial/PRICING.md).

Keep broad claims gated. Use `context_payload_reduction` for estimated prompt-payload savings, and use observed token reduction only when real paired transcripts provide audited token counts.

## Current State

CallSieve is now an open-source local Rust CLI with a JSON index, deterministic retrieval, optional local embeddings, optional LSP reference enrichment, CODEOWNERS and git-history signals, stack-trace-aware error context, lifecycle hooks for Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, and Cline, MCP/rule/template setup for Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, and Zoo Code, context-first guardrails, a daemon that keeps the index fresh and serves agent-context from memory over a local socket (0.31s vs 0.61s direct on a 2.7k-file repo, byte-identical output), team index warm starts via `index-export`/`index-import`, one-command `setup-auto` agent detection, benchmark reports, observed-session traces, and gated proof reports. `public-proof-report` now exposes 19/19 default-layer client setup coverage so the agent-neutral MCP story is checked in the same artifact as retrieval proof.

Language coverage is now broad enough for common multi-language repos: Python, TypeScript, JavaScript, C++, Java, C#, Go, Rust, SQL, Kotlin, Swift, Dart, PHP, Ruby, Bash, PowerShell, C, Scala, Elixir, Lua, Objective-C, Zig, Julia, OCaml, Haskell, and PL/SQL are indexed with lightweight symbol extraction and practical import/include detection. Tree-sitter still provides richer parsing for the proven first languages, while the long tail uses deterministic local heuristics that can improve without changing the zero-cloud architecture.

Public retrieval result (June 2026): deterministic Mode A first-correct-file@5 is `100.0%` on the 50-issue SWE-bench Lite subset in `benchmarks/public/manifest-50.json` (`benchmarks/public/results/mode-a-50-domain.json`), with zero remaining public misses in that checked slice. A third-repo 10-task `psf/requests` seed in `benchmarks/public/manifest.json` also passes at `100.0%` (`benchmarks/public/results/mode-a-requests-seed.json`), bringing the public proof gate to 186 evaluated public tasks across strict compare, deterministic, natural-language, and language-smoke reports. The stricter checked-in compare gate still proves `60.0%` lexical first-correct-file@5 with a `+54.0 pp` lift over the naive-grep baseline of `6.0%`, and an opt-in hybrid comparison reaches `60.0%` with 50 ties and zero losses. On the 30-issue natural-language slice in `benchmarks/public/manifest-nl.json`, deterministic Mode A now reaches `100.0%` first-correct-file@5 (`benchmarks/public/results/mode-a-nl-domain.json`) and the public proof target is `100.0%`; the stricter compare report still records `36.7%` hybrid with `+23.3 pp` over grep. Public pinned language-smoke slices also pass at `100.0%` first-correct-file@5 for Rust (`benchmarks/public/results/mode-a-rust-callsieve.json`) and TypeScript (`benchmarks/public/results/mode-a-typescript-callsieve.json`). Full-repo prompt-pack proxy baselines measure Astropy at `5,638,468` estimated tokens and Django at `5,155,623` estimated tokens, both over `1,000x` the default CallSieve proof packet; the smaller Requests checkout measures `101,186` estimated tokens, still over `20x` the compact packet. The mechanism came from checked-in measurements: Python settings/constants are indexed as symbols, common NL code vocabulary is bridged to path/symbol terms, and framework-domain module aliases connect human task language to implementation modules such as WCS, migrations, SQL compilers, SQL query builders, ORM lookups, autoreloaders, serializers, validators, enums, URL resolvers, SQLite test-database creation, SQL order-by relation compiler routing, SQL query-builder routing for filterability and combined-query issues, auth proxy-permission migration routing, Requests session method-normalization routing, Requests dependency-exception pass-through routing, `UniqueConstraint` model field-check routing, `ForeignKey` `to_field` rename autodetection, and dynamic `SCRIPT_NAME` static/media settings resolution.

For human installation and client setup, see [docs/INSTALL.md](docs/INSTALL.md). For AI CLI and wrapper behavior, see [docs/AGENT_CLI.md](docs/AGENT_CLI.md). For observed whole-session proof collection, see [docs/OBSERVED_SESSIONS.md](docs/OBSERVED_SESSIONS.md). For dogfooding and less-grep measurement, see [docs/DOGFOOD.md](docs/DOGFOOD.md). For paid pilot packaging, see [docs/PILOTS.md](docs/PILOTS.md). For competitive positioning and product gaps, see [docs/COMPETITIVE.md](docs/COMPETITIVE.md).

The core workflow is:

```text
index repo -> ask for agent context -> read returned files first -> grep only if needed -> audit traces and savings
```

## Try It In 60 Seconds

```bash
brew install philipjohnbasile/callsieve/callsieve   # or: cargo install --git https://github.com/PhilipJohnBasile/callsieve
callsieve demo /path/to/repo --task "find where login is handled"
callsieve hook install /path/to/repo --client generic --strict --force
callsieve hook doctor /path/to/repo
callsieve codex-hooks install /path/to/repo --strict --force
callsieve claude-hooks install /path/to/repo --strict --force
```

`demo` proves the retrieval loop without configuring an agent. `hook install` creates repo-local launchers and search shims under `.callsieve/` so testers can start an agent with CallSieve-first guardrails without changing global PATH or shell profiles. Hook-capable clients also get local project hooks or plugins that inject CallSieve context and block pre-context broad search after they are trusted.

`agent-context` defaults to a budgeted `skim` packet: up to five compact file, symbol, reason, related-test, risk, and one upstream/downstream non-test code-file `g` preview for the top read-first file with no snippets or call-path edges. It rebuilds a missing or stale local index before ranking, so direct CLI agents do local refresh work before spending model tokens on context. The token budget is enforced on the full agent-facing response, compacting local task-memory hints, trimming optional local-expansion commands, graph hints, and call paths, shrinking envelope metadata, then dropping selected files only if needed. Default skim uses array order instead of per-file `rank`, score, or `selection_confidence` fields, `f` for file, caps `sy` symbols at one per file, uses symbol arrays such as `["createSession",12]` with a trailing compact kind code such as `s` for `struct` only when it is not the implicit `function`, uses `w` only for top-file reasons not already repeated by `context.sel.top`, uses positional `i` arrays for impact as `[risk, tests, upstream, downstream, flags]` with risk coded as `l`, `m`, or `h`, read-first indexes in the `tests` slot when possible, and optional flags such as `test,im,call,ref,by` naming which compact graph evidence exists, uses compact `instruction.x` target keys without a non-action `instruction.a`, keeps default task-memory matching local without serializing a hit-only `memory` envelope, exposes `memory.f` and `memory.sy` only in richer profiles, omits default JSON `context.root` because the command or integration already supplies the repo, uses compact `context.sel` arrays for ranking explainability with short signal codes and read-first indexes for selected files, uses compact `g.u` and `g.d` graph preview keys only on the top file, omits related tests from `g` when `i` already carries them, omits default `cp` call paths until `focus` or a richer profile is requested, omits default git hints unless `--git-boost` is requested, omits the ordinary task echo, relies on path extensions instead of per-file `language`, uses short reason codes such as `sym:`, `sy:`, `kw:`, `ct:`, `pt:`, and `test:`, drops duplicate generic `kw:` when a matching `sy:` reason is present, and omits empty symbol/test/count fields. Output includes `retrieval_cost.retrieval_model_tokens = 0`, compact packet stats with `b` for budget and `t` for returned context tokens, and compact `stats.local` counts using `f`, `sy`, and `r` so agents can distinguish zero-token local retrieval from the context tokens spent reading the returned packet, while still seeing how many files, symbols, and references CallSieve searched on-machine. It also returns `context.sel.top` as `[index, why]`, capped `context.sel.sig` code strings, and one capped `context.sel.next` entry as `[index, why]` without repeating scores; literal path fallback keeps `[path, score, why]` only when a read-first index cannot be used. The packet also includes zero-based `instruction.x.o` and scalar `instruction.x.n` read-first indexes; legacy or wider packets may use `instruction.x.top` or `instruction.x.next`. Compact `instruction.x.r = 1` and `instruction.x.t = 1` mean run local `related` and `tests` follow-ups for the top file. Markdown and editor views expand these compact targets into copyable commands. Use `focus`, `related`, `--limit`, `--profile normal`, `--profile full`, or `--snippets-per-file` to reveal more graph detail only after the first packet is insufficient.

`callsieve focus <repo> --file <file> --symbol <symbol> --line <line>` is the preferred second step before any full-file read. Generated expansion commands include `--line` when CallSieve selected a symbol line, so same-name symbols stay disambiguated. Symbol focus returns the selected code unit as a bounded snippet up to 120 lines by default, with `truncated` and `omitted_lines` only when the local snippet cap is hit. It also includes compact `calls`, `called_by`, and `related_tests` hints for the selected symbol so agents can see local blast radius before running `related`, `tests`, or grep. Non-call `references` are opt-in with `--references` because they can be noisy. File-level focus without `--symbol` stays compact.

Optional retrieval signals stay opt-in:

```bash
cargo build --features embed
target/debug/callsieve index <repo> --embeddings
target/debug/callsieve agent-context <repo> "<task>" --embeddings --git-boost
target/debug/callsieve agent-context <repo> "<task>" --error <stacktrace.log>
```

`--embeddings` requires a binary built with `--features embed`; the default build stays deterministic and dependency-light. `--git-boost` nudges recently changed or high-churn files only when requested and keeps compact `git` hints in skim output. `--error` parses stack traces and error logs, then promotes indexed files named in the frames. The integer displayed score remains lexical and explainable; normal/full packets also expose `selection_confidence` as `high`, `medium`, or `low` relative to the top selected file. Hybrid retrieval only changes ordering when explicitly enabled.

JSON output is compact by default for agent token savings. Add global `--pretty` for human-readable formatting. Add global `--tokenizer o200k|cl100k` (requires a binary built with `--features tokenizers`) to count budget-enforcement and proof tokens with a real BPE tokenizer instead of the default `bytes/4` heuristic. Add global `--bm25` to enable opt-in BM25+ length normalization for the content-keyword ranking signal, and global `--pagerank` to add an opt-in PageRank graph-centrality boost so files the codebase structurally centers on rank higher; like `--embeddings` and `--git-boost` these only change ordering when explicitly enabled.

## Current CLI Surface

```bash
callsieve index <path> [--lsp] [--embeddings] [--embed-model small|code]
callsieve symbols <path>
callsieve symbol <path> <symbol_name>
callsieve query <path> "<question>" [--why-debug]
callsieve context <path> "<task>" [--limit <n>] [--snippets-per-file <n>] [--no-snippets] [--profile skim|normal|full] [--token-budget <n>] [--why-debug] [--format json|markdown]
callsieve agent-context <path> "<task>" [--limit <n>] [--snippets-per-file <n>] [--profile skim|normal|full] [--token-budget <n>] [--why-debug] [--embeddings] [--error <file>] [--git-boost] [--format json|markdown]
callsieve focus <path> --file <file> [--symbol <symbol>] [--line <line>] [--references] [--skeleton]
callsieve related <path> --file <file>
callsieve graph <path> --file <file> [--direction dependencies|dependents|both] [--depth <1-3>]
callsieve tests <path> --file <file>
callsieve demo <path> [--task "<task>"] [--lsp]
callsieve memory-clear <path>
callsieve benchmark <path> "<task>" [--limit <n>] [--snippets-per-file <n>] [--no-snippets] [--profile skim|normal|full] [--token-budget <n>]
callsieve benchmark-suite <path> <tasks.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets] [--profile skim|normal|full] [--token-budget <n>]
callsieve bench-public <manifest.json> <repos_dir> [--k <n>] [--out <path>] [--embeddings] [--compare]
callsieve bench-run <manifest.json> --workdir <dir> [--compare] [--k <n>] [--limit <n>] [--out <path>] [--resume]
callsieve eval-retrieval <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets] [--profile skim|normal|full] [--token-budget <n>] [--json]
callsieve perf-report <path> [--tasks <manifest.json>] [--iterations <n>] [--json]
callsieve trace-summary <trace.json>
callsieve session-start <path> "<task>" --client codex --model <name> --trace <trace.json> [--expected-file <path>] [--critical-file <path>]
callsieve session-event <trace.json> --command <cmd> [--files-read <path>...] [--context-selected-file <path>...] [--tokens <n>] [--phase baseline|callsieve]
callsieve session-finish <trace.json> --out <summary.json>
callsieve trace-replay <path> <tasks.json> <trace.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve trace-check <trace.json> [--strict]
callsieve benchmark-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve competitive-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve public-proof-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve repo-pack-baseline <path> [--id <id>] [--tool <name>] [--out <file>]
callsieve agent-native-template <path> <tasks-or-public-manifest.json> [--k <n>] [--limit <n>] [--snippets-per-file <n>] [--no-snippets] [--profile skim|normal|full] [--token-budget <n>] [--out <tasks.json>]
callsieve agent-native-check <tasks.json> [--mode template|measured] [--source-artifact <transcript-or-export>...] [--out <file>]
callsieve agent-native-protocol [--out <protocol.json>]
callsieve agent-native-baseline <tasks.json> [--id <id>] [--tool <name>] [--k <n>] [--measurement-command <cmd>] --source-artifact <transcript-or-export>... [--measurement-note <note>...] [--out <file>]
callsieve benchmark-doctor <manifest.json>
callsieve pilot-init <manifest.json> [--sessions <n>]
callsieve pilot-task add <manifest.json> <repo> "<task>" [--id <id>] [--expected-file <path>] [--critical-file <path>] [--external] [--pair-id <id>] [--task-category <name>] [--difficulty <name>] [--condition <name>] [--token-source transcript_context_tokens]
callsieve pilot-task reject <manifest.json> --task-id <id> --reason <reason>
callsieve pilot-run <manifest.json> --task-id <id> --mode baseline|callsieve --command <cmd> [--files-read <path>...] --tokens <n>
callsieve proof-sprint init <manifest.json> --client claude --sessions 10|50 --model <name> [--force]
callsieve proof-sprint status <manifest.json> [--json]
callsieve proof-sprint collect <manifest.json> --task-id <id> --mode baseline|callsieve [--max-budget-usd <n>] [--dry-run]
callsieve proof-sprint run <manifest.json> [--resume] [--limit <n>] [--max-budget-usd <n>] [--dry-run]
callsieve proof-sprint finalize <manifest.json> --out <proof.json> [--limit <n>]
callsieve pilot-collect-ollama <manifest.json> [--model qwen2.5-coder:7b] [--limit <n>] [--context-limit <n>]
callsieve pilot-collect-lm-studio <manifest.json> [--model qwen3-coder-next] [--base-url http://127.0.0.1:1234/v1] [--limit <n>] [--context-limit <n>]
callsieve pilot-qa <manifest.json>
callsieve pilot-finalize <manifest.json> --out <proof.json>
callsieve pilot-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve proof-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve enterprise-proof-report <manifest.json> [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve pilot-doctor <manifest.json>
callsieve evidence-pack <manifest.json> [--anonymize]
callsieve policy-check <trace.json> [--strict]
callsieve mcp
callsieve mcp-config <path> [--format json|toml]
callsieve mcp-registry-manifest [--out <server.json>]
callsieve mcp-contract [--out <mcp-contract.json>]
callsieve status <path>
callsieve daemon <path> [--background] [--foreground] [--once] [--lsp]
callsieve daemon-status <path>
callsieve daemon-stop <path>
callsieve watch <path> [--debounce-ms <n>] [--foreground] [--lsp]
callsieve agent-setup <path> --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> [--force]
callsieve setup-agent <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> <path> [--force]
callsieve setup-auto <path> [--force] [--dry-run]
callsieve index-export <path> --out <file>
callsieve index-import <path> --from <file> [--allow-partial]
callsieve memory-export <path> --out <file> [--format json|mxf]
callsieve memory-import <path> --from <file> [--format json|mxf]
callsieve memory-pin <path> --task "<task>" [--unpin]
callsieve receipt <path> [--session <id>] [--format json|markdown]
callsieve receipts <path>
callsieve bootstrap <path> --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> [--strict] [--force] [--lsp]
callsieve doctor <path> --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> [--fix] [--strict]
callsieve codex-bootstrap <path> --model <name> [--force]
callsieve codex-hooks install <path> [--strict] [--force] [--limit <n>] [--snippets-per-file <n>] [--lsp]
callsieve codex-hooks doctor <path> [--strict] [--smoke] [--fix]
callsieve codex-hooks trust-ack <path>
callsieve codex-hooks uninstall <path>
callsieve claude-hooks install <path> [--strict] [--force] [--limit <n>] [--snippets-per-file <n>] [--lsp]
callsieve claude-hooks doctor <path> [--strict]
callsieve claude-hooks uninstall <path>
callsieve copilot-hooks|opencode-hooks|antigravity-hooks|cline-hooks install <path> [--strict] [--force] [--limit <n>] [--snippets-per-file <n>] [--lsp]
callsieve copilot-hooks|opencode-hooks|antigravity-hooks|cline-hooks doctor <path> [--strict]
callsieve copilot-hooks|opencode-hooks|antigravity-hooks|cline-hooks uninstall <path>
callsieve editor-hook <path> --editor <vscode|cursor|generic> [--force]
callsieve hook install <path> --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> [--strict] [--force] [--lsp]
callsieve hook doctor <path>
callsieve hook uninstall <path>
callsieve guard <path> "<task>" [--trace-out <trace.json>]
callsieve begin <path> "<task>" --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> [--trace-out <trace.json>] [--proof-trace]
callsieve codex-session <path> "<task>" --trace-out <trace.json> [--model <name>] [--expected-file <path>]
callsieve enforce <path> --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> [--trace <trace.json>] [--strict] [--require-shim]
callsieve shim install <path> [--force] [--strict]
callsieve shim doctor <path>
callsieve shim uninstall <path>
callsieve grep <path> "<query>" [--run-rg] [--structural <ast-grep-pattern>] [--structural-lang <lang>]
callsieve stats <path>
```

Example:

```bash
cargo run -- index .
cargo run -- index . --lsp
cargo run -- demo . --task "change login token expiry behavior"
cargo run -- query . "where is auth handled?"
cargo run -- context . "change login token expiry behavior"
cargo run -- agent-context . "change login token expiry behavior"
cargo run -- agent-context . "change login token expiry behavior" --format markdown
cargo run -- memory-clear .
cargo run -- benchmark . "change login token expiry behavior"
cargo run -- benchmark-suite . benchmarks/tasks.json
cargo run -- benchmark-suite . benchmarks/callsieve-real-repo.json
cargo run -- eval-retrieval benchmarks/retrieval-fixtures.json
cargo run -- perf-report . --iterations 5
cargo run -- proof-rehearsal --fix --resume
cargo run -- trace-summary benchmarks/session-trace.example.json
cargo run -- session-start . "change login token expiry behavior" --client codex --model gpt-5-codex --trace .callsieve/observed-session.json
cargo run -- session-event .callsieve/observed-session.json --command "callsieve agent-context . \"change login token expiry behavior\"" --context-selected-file src/auth/session.ts --tokens 3000 --phase callsieve
cargo run -- session-finish .callsieve/observed-session.json --out .callsieve/observed-summary.json
cargo run -- trace-replay . benchmarks/callsieve-real-repo.json benchmarks/session-trace.local.json --limit 20
cargo run -- trace-check benchmarks/session-trace.example.json --strict
cargo run -- benchmark-report benchmarks/report-manifest.example.json
cargo run -- competitive-report benchmarks/competitive-response-manifest.example.json
cargo run -- public-proof-report benchmarks/public-proof-manifest.example.json
cargo run -- benchmark-doctor benchmarks/report-manifest.example.json
cargo run -- pilot-init benchmarks/evidence/pilot.local.json --sessions 1
cargo run -- pilot-task add benchmarks/evidence/pilot.local.json . "change login token expiry behavior" --id auth-expiry --expected-file src/auth/session.ts --critical-file src/auth/session.ts
cargo run -- pilot-run benchmarks/evidence/pilot.local.json --task-id auth-expiry --mode baseline --command "rg login token expiry" --files-read src/auth/session.ts --tokens 12000
cargo run -- pilot-run benchmarks/evidence/pilot.local.json --task-id auth-expiry --mode callsieve --command "callsieve agent-context . \"change login token expiry behavior\"" --files-read src/auth/session.ts --tokens 3000
cargo run -- record-codex-observed-session --manifest benchmarks/evidence/pilot.local.json --task-id auth-expiry --mode callsieve --command "callsieve agent-context . \"change login token expiry behavior\"" --tokens 3000 --files-read src/auth/session.ts
cargo run -- record-observed-session --manifest benchmarks/evidence/pilot.local.json --client claude --model claude-opus-4-8 --task-id auth-expiry --mode callsieve --command "claude -p \"change login token expiry behavior\" --output-format json" --usage-json .callsieve/observed/auth-expiry-callsieve.json --files-read src/auth/session.ts
cargo run -- collect-claude-observed-session --manifest benchmarks/evidence/observed-claude-oss-50.local.json --task-id ripgrep-ignore-walk-claude-r01 --mode callsieve --context-limit 4 --snippets-per-file 0 --max-budget-usd 0.50
cargo run -- proof-sprint init benchmarks/evidence/proof-sprint.local.json --client claude --sessions 10 --model claude-opus-4-8
cargo run -- proof-sprint status benchmarks/evidence/proof-sprint.local.json
cargo run -- proof-sprint collect benchmarks/evidence/proof-sprint.local.json --task-id ripgrep-ignore-walk-claude-r01 --mode baseline --dry-run
cargo run -- proof-sprint run benchmarks/evidence/proof-sprint.local.json --resume --dry-run
cargo run -- proof-sprint finalize benchmarks/evidence/proof-sprint.local.json --out benchmarks/evidence/proof.local.json
cargo run -- pilot-collect-ollama benchmarks/evidence/observed-generic-ollama-100.local.json --model qwen2.5-coder:7b --limit 10 --context-limit 24
cargo run -- pilot-collect-lm-studio benchmarks/evidence/observed-generic-ollama-100.local.json --model qwen3-coder-next --base-url http://127.0.0.1:1234/v1 --limit 10 --context-limit 24
cargo run -- pilot-qa benchmarks/evidence/pilot.local.json
cargo run -- pilot-finalize benchmarks/evidence/pilot.local.json --out benchmarks/evidence/proof.local.json
cargo run -- pilot-report benchmarks/pilot-manifest.example.json
cargo run -- proof-report benchmarks/pilot-manifest.example.json
cargo run -- enterprise-proof-report benchmarks/evidence/enterprise-proof-manifest.example.json
cargo run -- pilot-doctor benchmarks/pilot-manifest.example.json
cargo run -- evidence-pack benchmarks/pilot-manifest.example.json --anonymize
cargo run -- policy-check benchmarks/session-trace.example.json --strict
cargo run -- mcp
cargo run -- mcp-config . --format json
cargo run -- mcp-registry-manifest --out server.json
cargo run -- mcp-contract --out benchmarks/public/results/mcp-contract.json
cargo run -- agent-native-protocol --out benchmarks/public/results/agent-native-protocol.json
cargo run -- agent-native-template benchmarks/public/repos/psf/requests benchmarks/public/manifest.json --k 5 --out benchmarks/public/results/agent-native-requests-template.json
cargo run -- agent-native-check benchmarks/public/results/agent-native-requests-template.json --mode measured --source-artifact benchmarks/public/results/cursor-public-requests-transcript.json --out benchmarks/public/results/cursor-public-requests-check.json
cargo run -- status .
cargo run -- daemon . --once
cargo run -- daemon-status .
cargo run -- watch .
cargo run -- watch . --lsp
cargo run -- agent-setup . --client codex
cargo run -- bootstrap . --client generic --strict --force
cargo run -- doctor . --client generic --strict
cargo run -- doctor . --client generic --fix --strict
cargo run -- codex-bootstrap . --model gpt-5-codex --force
cargo run -- codex-hooks install . --strict --force
cargo run -- codex-hooks doctor . --strict --smoke
cargo run -- codex-hooks trust-ack .
cargo run -- hook install . --client claude --strict --force --lsp
cargo run -- claude-hooks doctor . --strict
cargo run -- editor-hook . --editor cursor --force
cargo run -- hook install . --client generic --strict --force --lsp
cargo run -- hook doctor .
cargo run -- hook uninstall .
cargo run -- guard . "change login token expiry behavior" --trace-out .callsieve/session-trace.json
cargo run -- begin . "change login token expiry behavior" --client generic --trace-out .callsieve/session-trace.json --proof-trace
cargo run -- codex-session . "change login token expiry behavior" --trace-out .callsieve/codex-session.json --model gpt-5-codex
cargo run -- enforce . --client codex --trace .callsieve/session-trace.json --strict
cargo run -- shim install . --force --strict
cargo run -- shim doctor .
cargo run -- grep . "change login token expiry behavior"
```

Reject invalid observed runs without deleting their audit trail:

```bash
cargo run -- pilot-task reject benchmarks/evidence/pilot.local.json --task-id auth-expiry --reason "operator learned answer during paired run"
```

For the current 50-session observed Codex milestone, preregister the six-repo OSS task matrix with:

```bash
cargo run -- setup-observed-codex-oss-50
cargo run -- setup-observed-claude-oss-50 --model claude-opus-4-8
```

Run the deterministic rehearsal separately:

```bash
cargo run -- proof-rehearsal --preflight
cargo run -- proof-rehearsal --fix --resume
```

The Rust rehearsal command is self-healing for local-safe issues. `--preflight` validates prerequisites, `--fix` rebuilds local indexes, creates ignored evidence directories, and regenerates missing controlled traces, `--resume` skips already-passed matching steps from `benchmarks/evidence/rehearsal-run.local.json`, and `--retry-count` retries transient failures. Output is JSON by default. It never clones repos, installs tools, deletes evidence, records observed sessions, or runs `proof-report`.

`pilot-init` defaults to the strict 100-session claim protocol. Use `--sessions 1` only for local workflow shakedowns.

## Current Capabilities

- walks a repository while respecting common ignore rules
- detects TypeScript, JavaScript, Python, Rust, PHP, Go, Java, C#, C, C++, Ruby, Kotlin, Swift, Scala, Dart, Lua, and shell source files plus agent-relevant docs and config files
- extracts practical symbols with tree-sitter-backed parsing and deterministic fallbacks
- extracts imports, references, and calls
- can enrich references with local Language Server Protocol servers when `--lsp` is enabled
- can build an optional local embeddings cache with `--embeddings` for opt-in hybrid retrieval
- indexes CODEOWNERS ownership and recent git activity for local ranking and context signals
- can parse stack traces through `agent-context --error` to promote crash-related files
- indexes bounded content terms for Markdown, JSON, TOML, YAML, and text without returning full files
- stores a local JSON index at `.callsieve/index.json`
- returns compact JSON for agent consumption, with Markdown output available for direct reading
- ranks matches with deterministic, explainable scoring
- builds compact read-first context packets for coding tasks
- boosts package manifests for dependency and setup tasks
- boosts context with import, caller, and callee proximity
- provides an `agent-context` wrapper agents can call before grep
- keeps a small local task-memory hint cache for repeated task families
- clears local task memory with `memory-clear` for cold-run testing
- runs a `demo` command that indexes, returns read-first files, and reports context payload reduction
- installs lifecycle hooks or plugins for Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, and Cline
- exposes a minimal MCP stdio server so agents can call CallSieve before grep
- prints portable JSON/TOML MCP configs for generic AI CLIs with `mcp-config` and a local-first MCP Registry `server.json` descriptor with `mcp-registry-manifest`
- reports platform-neutral `context_payload_reduction` versus a naive grep/read loop
- evaluates retrieval recall against expected and critical file fixtures with `eval-retrieval`
- reports local p50/p95 context latency with `perf-report`
- records real observed agent session events and summarizes baseline versus CallSieve-assisted phases
- keeps controlled replay evidence separate from observed-session evidence
- aggregates benchmark evidence across multiple local repositories
- produces pilot and top-level proof reports that combine benchmark, observed trace, controlled replay, policy, freshness, bootstrap, daemon, and LSP evidence
- produces an opt-in enterprise proof report that gates broad claims on 1,000 observed sessions, multi-client coverage, scale proxies, strict trace policy, and PMF evidence
- produces anonymized evidence packs for external pilot aggregation
- provides CI-friendly strict policy checks for context-first sessions
- validates evidence manifests before reports
- reports index freshness and keeps indexes fresh with a local watcher or daemon state loop
- generates client-specific agent rules that require CallSieve before broad grep
- writes an explicit first command into agent setup: `callsieve agent-context <repo> "<task>"`
- bootstraps the local adoption stack with `bootstrap`: index, agent config, daemon state, and optional strict shims
- audits and repairs local adoption setup with `doctor --fix`
- starts lightweight task sessions with `begin`, returning context and optionally writing a trace stub for strict audits
- guards context-first sessions and can write trace stubs for policy audits
- starts controlled Codex/ChatGPT context-first replay traces with model tags
- bootstraps project-local hooks, resolved MCP config, rules, and grep shims without global PATH/profile mutation
- generates project-local MCP configs, rules, and editor hook templates for supported agents without mutating global user config
- installs repo-local agent launchers with `hook install` so shims and daemon startup stay process-local
- audits agent setup, traces, index freshness, and optional shim state with `enforce`
- installs an opt-in local `callsieve` launcher plus `rg`/`grep` shims for PATH-level interception
- wraps grep workflows so CallSieve context is returned before the original `rg` or `grep` command is replayed
- runs a resumable public 50-issue lexical-vs-hybrid A/B benchmark with `bench-run --compare --resume`

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
  "root": ".",
  "retrieval_cost": {
    "retrieval_model_tokens": 0
  },
  "sel": {
    "top": [0, "sym:createSession"],
    "sig": ["sym"],
    "next": [[1, "test:related"]]
  },
  "read_first": [
    {
      "f": "src/auth/session.ts",
      "sy": [["createSession", 12]],
      "i": ["m", "src/auth/session.test.ts", 1, 1],
      "g": {
        "u": ["src/auth/token.ts"]
      }
    }
  ],
  "stats": {
    "b": 1200,
    "t": 900,
    "local": {
      "f": 182,
      "sy": 1200,
      "r": 5000
    }
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

`benchmark-suite` reports expected-file recall, first-correct-file@k, aggregate `context_payload_reduction`, legacy estimated token-savings fields, and optional observed session savings when real agent trace numbers are supplied. `context_payload_reduction` is the platform-neutral proxy for Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo Code, the deprecated Roo alias, generic stdio MCP tools, and local agents. It estimates only the prompt context payload CallSieve controls, not whole-session transcript tokens.

`eval-retrieval` runs the same task fixture shape against the actual `agent-context` selection path and reports recall@k, first-correct-file@k, critical recall, selected tokens, and failure reasons. It exits nonzero when a critical file is missed. `perf-report` runs fixed local tasks and reports p50/p95 latency for index load plus context generation.

`trace-replay` generates deterministic baseline versus CallSieve trace JSON from a suite. It is tagged with `metadata.collection = "controlled_replay"` and is useful before real observed session evidence exists.

Use `session-start`, `session-event`, and `session-finish` for real observed agent sessions across hook-capable and MCP-capable clients, including Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo Code, and local agents. These traces are tagged with `metadata.collection = "observed_session"` and keep ordered events with command classification, actual files read, CallSieve-selected context files, optional token counts, and phase (`baseline` or `callsieve`).

`files_read` means actual file-read tool or command paths. Use `context_selected_files` or `--context-selected-file` for files selected into a CallSieve read-first packet when no whole-file read happened. Only `metadata.collection = "observed_session"` is counted as observed proof. Lifecycle collections such as `codex_hook_trace`, `claude_hook_trace`, and `<client>_hook_trace` are guardrail telemetry and are excluded from observed proof gates.

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

`competitive-report` turns the competitive goal in [docs/COMPETITIVE.md](docs/COMPETITIVE.md) into a repeatable local evidence artifact. It reads a manifest such as [benchmarks/competitive-response-manifest.example.json](benchmarks/competitive-response-manifest.example.json), runs CallSieve's benchmark, natural-language benchmark, strict trace-policy check, and optional perf evidence locally, and emits a comparison matrix for install friction, first-query time, code upload requirement, retrieval model-token cost, per-query cost, packet tokens, first-correct-file@5, expected-file recall, natural-language evidence, explainability, agent coverage, and trace or receipt proof. Competitor rows are source claims; CallSieve rows are locally measured. Missing natural-language, required-agent, first-mile economics, performance, or context-first trace evidence is reported as a warning or failure depending on manifest targets.

`public-proof-report` is the public-facing proof track. It runs the local competitive gate, reads checked-in public benchmark reports, enforces public first-correct and grep-lift targets, keeps zero retrieval model tokens / zero retrieval cost / no default upload as hard invariants, and emits an evidence pack with commands, fixtures, metrics, a checked-in public result catalog, sampled misses, receipt commands, terminal artifacts, agent setup/MCP evidence, a `broad_read_guardrail`, a `context_packet_guardrail`, optional miss triage, measured full-repo pack or repo-packer output-size baselines, and optional agent-native search baselines. Fresh `bench-run` reports include a retrieval contract fingerprint; `public-proof-report` marks every checked public report as `current`, `missing`, or `stale` and fails stale reports so old ranker output cannot silently support a claim. The checked public manifest sets `require_current_public_report_retrieval_contract: true`, so deterministic public claim reports must be regenerated by the current retrieval contract; legacy compare reports can explicitly opt out while remaining useful as grep-lift evidence. The broad-read guardrail fails closed when CallSieve's measured context packet does not beat the naive broad file-read payload by the manifest target. The context-packet guardrail fails closed when the local competitive packet lacks selected files, symbols, related tests, blast-radius/risk hints, caller/callee call-graph hints, selection evidence, or local focus/related/tests expansion targets. The MCP proof includes an `mcp_surface` block that fails closed if `callsieve_context`, `callsieve_focus`, `callsieve_related`, `callsieve_tests`, or `callsieve_status` are missing from the MCP tools list, plus an `mcp_contract` block for the stable `structuredContent` fields, follow-up instruction keys, freshness fields, default skim profile, and default token budget agents consume without custom glue. The checked standalone `mcp-contract.json` artifact is content-validated against the live `callsieve mcp-contract` output. The checked `agent-native-protocol.json` artifact is content-validated against the live `callsieve agent-native-protocol` output, so the native-search measurement playbook cannot drift from the CLI. The checked `agent-native-check.json` artifact is content-validated by recomputing a passing measured preflight from the referenced task log plus transcript/export source artifacts, with all task counters ready and every task marked `recording_status: "measured"`. Any measured agent-native baseline requires both the protocol and measured-check artifacts to pass, and the baseline task-log plus transcript/export hashes must be present together in one measured-check artifact before public proof can pass. The checked agent-native template artifact is validated by recomputing the unmeasured CallSieve task payload from the current source tasks, local index, and retrieval contract, so hand-edited `callsieve_files` or packet-token counts fail closed. Required pack and agent-native baselines fail closed when measured artifacts are missing, lack locally readable hashed transcript/export provenance, lack the checked protocol or measured preflight artifact, are not hash-linked to a single preflight artifact, or fall below configured thresholds; public proof rereads the listed source artifacts and recomputes their byte counts and hashes. Use `repo-pack-baseline` for the safe local full-repo prompt-pack proxy, add external Repomix, Gitingest, or Code2Prompt JSON artifacts when those tools are installed and approved, use `agent-native-template` to pre-fill CallSieve's side of Cursor/Copilot/Claude/Devin-style task logs, run `agent-native-check --mode measured --out <check.json>` with the transcript/export source artifacts to catch incomplete logs, and use `agent-native-baseline` with at least one `--source-artifact` transcript or native-search export to turn real approved native-search measurements into the standard guardrail artifact before adding them to `agent_native_search_baselines`. See [docs/PUBLIC_PROOF.md](docs/PUBLIC_PROOF.md) and [benchmarks/public-proof-manifest.example.json](benchmarks/public-proof-manifest.example.json).

## Example Pilot Report Manifest

```json
{
  "thresholds": {
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": 50.0,
    "minimum_observed_sessions": 1,
    "minimum_observed_token_reduction_percent": 50.0,
    "minimum_external_repos": 0,
    "minimum_planned_tasks": 1,
    "maximum_controlled_replay_ratio": 0.25,
    "maximum_trace_violations": 0,
    "maximum_critical_misses": 0,
    "require_fresh_index": true,
    "require_lsp_where_available": false,
    "require_codex_bootstrap": false,
    "require_transcript_token_accounting": false
  },
  "audit": {
    "planned_tasks": 1,
    "rejected_sessions": 0,
    "token_accounting_sources": ["transcript_context_tokens"]
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

`proof-report` is the top-level claim artifact. It exposes planned tasks, rejected-session audit count, observed sessions, transcript-token provenance, controlled replay sessions, external repo coverage, observed token reduction, controlled replay ratio, freshness, daemon, bootstrap, and LSP status in one JSON object. Controlled replay is never counted as observed evidence.

`proof-sprint` is the buyer-facing Claude Code wrapper around the same pilot evidence system. Use it for a 10-session paid-pilot proof sprint or the 50-session public claim gate when you want one command group for init, status, collection, resume, and finalization. `proof-sprint run --resume` collects the next missing baseline or CallSieve phase and stops when the target is met. It still refuses claim proof until `pilot-qa` passes.

`enterprise-proof-report` is the broad-claim artifact. It is opt-in and fails unless the manifest meets the enterprise gates: 1,000 paired observed sessions, 50 repos, 10 Microsoft-scale OSS proxies, manifest-configured client coverage, 5 languages, 10 task categories, 90% positive per-session savings, 75% of sessions above 30% savings, zero critical misses, zero strict trace violations, zero controlled replay, full transcript token accounting, and paid-pilot PMF evidence. See [docs/ENTERPRISE_PROOF.md](docs/ENTERPRISE_PROOF.md) and [benchmarks/evidence/enterprise-proof-manifest.example.json](benchmarks/evidence/enterprise-proof-manifest.example.json).

Evidence is separated into three tiers:

- Rehearsal: deterministic retrieval fixtures, benchmark reports, perf reports, platform-neutral `context_payload_reduction`, and controlled replay traces.
- Supplemental: Ollama or local-model collection, useful for rehearsal but not Codex claim proof.
- Claim-counted: real paired Codex sessions with transcript token accounting, transcript-backed files read, strict policy checks, and `pilot-qa` passing before `proof-report`.

Use `context_payload_reduction` when comparing CallSieve across agent platforms. Use observed token reduction only when a real paired transcript provides audited token counts.

`proof-report` should only be used as claim proof after `pilot-qa` passes for the claim-counted manifest.

For the strict observed-session collection protocol, see [docs/OBSERVED_SESSIONS.md](docs/OBSERVED_SESSIONS.md). The 50-session milestone is the first credible public proof target; the enterprise proof target remains gated at 1,000 paired observed sessions.

Use `evidence-pack` when you need a shareable aggregate for external pilots:

```bash
cargo run -- evidence-pack benchmarks/pilot-manifest.example.json --anonymize
```

With `--anonymize`, repo paths, labels, suite paths, and trace paths are redacted while aggregate metrics remain intact.

## Agent Enforcement

Use `bootstrap` when you want the whole local adoption stack in one command: rebuild the index, write client-specific MCP config and policy files, start the daemon state path, and optionally install strict local shims. It does not mutate global shell profiles, global user PATH, or cloud configuration.

```bash
cargo run -- bootstrap . --client generic --strict --force --lsp
cargo run -- doctor . --client generic --strict
cargo run -- doctor . --client generic --fix --strict
```

`doctor` reports local adoption checks: fresh index, generated agent files, optional Codex bootstrap files, strict shim files, and whether the shim directory is currently on the agent shell PATH. With `--fix`, it repairs missing or stale local pieces it can safely write under the repo. PATH changes remain an explicit shell choice for the launched agent process.

Use `begin` as the lightweight entrypoint for a task session. It returns the normal read-first context packet and, with `--trace-out`, writes the first context event so `trace-check --strict` can audit later grep or file reads:

```bash
cargo run -- begin . "change login token expiry behavior" --client generic --trace-out .callsieve/session-trace.json --proof-trace
cargo run -- trace-check .callsieve/session-trace.json --strict
```

Add `--proof-trace` when the trace is used as claim evidence. It labels the trace as explicit session events and does not depend on Codex `PostToolUse`. After a proof trace starts, every added `session-event` must include `--tokens` and explicit `--phase baseline|callsieve`.

Use `agent-setup` when you only need local MCP config plus a short CallSieve-first policy file for Codex, Claude, Copilot, OpenCode, Antigravity, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo, Roo alias, or generic MCP clients:

```bash
cargo run -- agent-setup . --client codex --force
```

For coding tasks, the policy is: call `callsieve_context` before broad grep, `rg`, repository-wide search, or repeated file reads. Read `read_first` files, review the capped `context.sel.next` target, use `instruction.x.o/n/next` plus local `focus`, `related`, and `tests` expansion commands for targeted detail, then grep only if the context packet and local expansion are insufficient.

Use `hook install` when you want the easiest repo-local agent entrypoint. It builds the index, writes client setup, installs strict shims, and creates `.callsieve/agent-launch.ps1` plus `.callsieve/agent-launch.sh`. For hook-capable clients, it also writes local project hooks or plugins. Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, and Zoo use MCP/rules/templates plus shims only.

```bash
cargo run -- hook install . --client generic --strict --force --lsp
cargo run -- hook doctor .
```

Hook setup does not mutate global shell profiles or user PATH. Remove the repo-local launchers and shims with:

```bash
cargo run -- hook uninstall .
```

For Codex, lifecycle hooks are the primary enforcement path:

```bash
cargo run -- codex-hooks install . --strict --force
cargo run -- codex-hooks doctor . --strict --smoke
cargo run -- codex-hooks trust-ack .
```

Codex hooks use the `slim` profile. The generated `.codex/hooks.json` runs local `callsieve codex-hook ...` handlers. `UserPromptSubmit` rebuilds a missing or stale local index, then injects compact `skim` CallSieve context plus local `focus`, `related`, and `tests` expansion commands. `PreToolUse` blocks broad search before context, and `PermissionRequest` denies escalated pre-context search. Codex `PostToolUse` and `Stop` are intentionally not installed because pre-tool hooks enforce the policy and post-tool or stop-time prompts are optional. Run `codex-hooks doctor --strict --smoke` for local handler smoke tests, and add `--fix` to archive stale hook state or trace files under `.callsieve/codex-hooks/archive/`. Review and trust project hooks in Codex with `/hooks`, then run `codex-hooks trust-ack .` to record a local marker tied to the current hook file hash.

For Claude Code, `hook install --client claude` gives all three local layers: hooks, shims, and MCP:

```bash
cargo run -- hook install . --client claude --strict --force --lsp
cargo run -- claude-hooks doctor . --strict
cargo run -- enforce . --client claude --strict
```

This writes `.mcp.json`, `CLAUDE.md`, `.claude/settings.local.json`, `.callsieve/agent-launch.ps1`, `.callsieve/agent-launch.sh`, and `.callsieve/bin/*`. The Claude Code hooks run local `callsieve claude-hook ...` handlers, inject compact `skim` context plus local expansion commands at `UserPromptSubmit`, block `Bash`, `Read`, `Grep`, and `Glob` before context in strict mode, and record `.callsieve/claude-hooks/*.trace.json`. Review and trust project hooks in Claude Code with `/hooks`.

For GitHub Copilot, OpenCode, Antigravity CLI, and Cline, use the same lifecycle pattern:

```bash
cargo run -- hook install . --client copilot --strict --force --lsp
cargo run -- copilot-hooks doctor . --strict
cargo run -- hook install . --client opencode --strict --force --lsp
cargo run -- hook install . --client antigravity --strict --force --lsp
cargo run -- hook install . --client cline --strict --force --lsp
```

Copilot writes `.github/copilot-instructions.md`, `.github/agents/callsieve-context.agent.md`, `.github/copilot-mcp.json`, and `.github/hooks/callsieve.json`. OpenCode writes `opencode.json`, `.opencode/CALLSIEVE.md`, and `.opencode/plugins/callsieve.js`. Antigravity writes `.agents/mcp_config.json`, `.agents/hooks.json`, `.agents/skills/callsieve-context.md`, and `.agents/rules/callsieve.md`. Cline writes `.cline/mcp.json`, `.cline/rules/callsieve.md`, `.clinerules/callsieve.md`, and `.cline/hooks/*`. Copilot cloud agents are template-only unless the local `callsieve` binary is installed inside the sandbox.

For VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, and Warp, CallSieve writes MCP/rule/skill/setup templates only. Strict mode requires those generated files, a fresh index, daemon state, and local shims, but it does not require lifecycle hooks. Global or user config files are not mutated automatically. Warp cloud-agent templates work only when the Warp/Oz runtime can execute the local `callsieve` binary.

Use `guard` to start a context-first task and write a trace stub, then use strict `trace-check` to audit actual sessions:

```bash
cargo run -- guard . "change login token expiry behavior" --trace-out .callsieve/session-trace.json
cargo run -- trace-check .callsieve/session-trace.json --strict
cargo run -- policy-check .callsieve/session-trace.json --strict
cargo run -- enforce . --client codex --trace .callsieve/session-trace.json --strict
```

`policy-check` exits nonzero when a trace violates the context-first rule, so it can be used in CI. `enforce` checks generated agent files, index freshness, optional trace policy, hook surfaces where supported, and shim state. In strict mode, first-class named clients require local shim files; generic clients can still opt into failing on missing shims with `--require-shim`.

For Codex/ChatGPT controlled replay, use `codex-session` instead of a generic guard. It writes a trace with `client: codex-chatgpt`, a model label, a deterministic grep/read baseline, and a CallSieve-first assisted side:

```bash
cargo run -- codex-session . "change login token expiry behavior" --trace-out .callsieve/codex-session.json --model gpt-5-codex
cargo run -- trace-summary .callsieve/codex-session.json
cargo run -- enforce . --client codex --trace .callsieve/codex-session.json --strict
```

Run the same task with different `--model` labels when comparing available Codex/ChatGPT models. CallSieve records and audits the sessions you run; it does not invoke hidden ChatGPT models itself.

`codex-session` is controlled replay evidence. For real observed sessions, use:

```bash
cargo run -- session-start . "change login token expiry behavior" --client codex --model gpt-5-codex --trace .callsieve/observed-session.json
cargo run -- session-event .callsieve/observed-session.json --command "callsieve agent-context . \"change login token expiry behavior\"" --context-selected-file src/auth/session.ts --tokens 3000 --phase callsieve
cargo run -- session-finish .callsieve/observed-session.json --out .callsieve/observed-summary.json
```

Use `codex-bootstrap` for Codex-first project setup without mutating global shell profiles or user PATH:

```bash
cargo run -- codex-bootstrap . --model gpt-5-codex --force
```

It writes `.codex/config.toml`, `.codex/CALLSIEVE.md`, `.codex/hooks.json`, `.callsieve/bin` launchers/shims, and `.callsieve/codex-launch.ps1` / `.callsieve/codex-launch.sh`. The MCP config points at the resolved CallSieve executable instead of relying on a global PATH entry. The lifecycle hooks inject context and block broad search before context. The launchers start `callsieve daemon --background --lsp`, prepend `.callsieve/bin` only for that launched process, and print the first required `callsieve agent-context` command.

This repo includes `benchmarks/codex-chatgpt-manifest.example.json` as the Codex pilot manifest fixture. Copy it to an ignored `.local.json` path before recording local runs.

For hard opt-in grep interception, install local wrappers and prepend `.callsieve/bin` to the agent shell PATH:

```bash
cargo run -- shim install . --force --strict
cargo run -- shim doctor .
```

The install writes a project-local `callsieve` launcher plus wrappers that call the hidden `callsieve shim-run` helper before passing through to the real `rg` or `grep` command captured at install time. `shim-run` parses common search arguments, rebuilds a missing or stale local index if needed, returns a compact `skim` CallSieve context packet first, then replays the original command arguments against the real search binary. With `--strict`, shim-mediated grep writes `.callsieve/shim-trace.json` events that strict trace checks can flag when grep happens before CallSieve context. The wrappers are inert until `.callsieve/bin` is prepended to the agent shell PATH for that process.

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
- PHP: `intelephense --stdio`
- Go: `gopls`
- C/C++: `clangd`
- Ruby: `ruby-lsp`
- Lua: `lua-language-server`
- C#: `csharp-ls`
- Java: `jdtls`
- Kotlin: `kotlin-language-server`
- Swift: `sourcekit-lsp`
- Scala: `metals`
- Dart: `dart language-server --protocol=lsp`

If a server is missing or fails, CallSieve keeps the tree-sitter and heuristic graph and reports per-language availability plus failure reasons in `status`. LSP-derived edges use sources such as `"lsp_reference"`, `"lsp_definition"`, `"lsp_implementation"`, and `"lsp_type_definition"` with `"confidence": 1.0`; tree-sitter edges use `0.8`, and heuristic edges use `0.5`.

## MCP Integration

`callsieve mcp` runs a stdio JSON-RPC server with these tools:

- `callsieve_context`: zero-AI-model-token local retrieval; build the compact read-first packet for a coding task before grep
- `callsieve_symbol`: find indexed symbols with import and reference hints
- `callsieve_focus`: reveal targeted symbols and snippets for one selected file
- `callsieve_related`: reveal imports, callers, callees, and blast-radius hints for one selected file
- `callsieve_graph_neighbors`: walk the import/reference graph from a file up to 3 hops (dependencies, dependents, or both) to explore blast radius
- `callsieve_tests`: reveal tests likely related to one selected file
- `callsieve_stats`: inspect index coverage
- `callsieve_status`: inspect freshness, watch, schema, and LSP enrichment state
- `callsieve_trace_check`: audit whether a session grepped before CallSieve
- `callsieve_benchmark`: estimate platform-neutral context payload reduction against a grep/read loop
- `callsieve_memory_recall`: recall similar past tasks and their read-first files from local task memory (Agent Memory Protocol verb `amp.recall`; read-only)
- `callsieve_memory_stats`: summarize local task memory: entry count, contributing clients, last task (`amp.stats`)
- `callsieve_memory_export`: export task memory in the vendor-neutral Memory Exchange Format (mxf) or native json (`amp.export`)
- `callsieve_memory_import`: merge an mxf or json memory document into local task memory (`amp.import`)
- `callsieve_memory_forget`: clear this repo's local task memory (`amp.forget`)
- `callsieve_memory_pin`: pin or unpin remembered tasks so they survive the eviction cap (`amp.pin`)

The memory verbs map CallSieve's local task-memory cache onto Agent Memory Protocol verb names and return Agent-Memory-Protocol-style error codes (`AMP_INVALID_ARGUMENT`, `AMP_INTERNAL`), and the Memory Exchange Format keeps export/import portable across agent-memory tools rather than CallSieve-proprietary.

`callsieve_context` self-heals a missing or stale `.callsieve/index.json` by rebuilding the local index before returning context. The direct CLI context-first commands do the same for `agent-context`, `context`, `begin`, `guard`, `codex-session`, and `grep`. MCP responses include freshness and timing metadata. The MCP server does not install shims, mutate client config, start the daemon, or send code to a remote service.

Use `callsieve mcp-config <repo> --format json` or `--format toml` for any AI CLI that supports stdio MCP but does not have a dedicated CallSieve setup command. Use `callsieve mcp-registry-manifest --out server.json` to generate a local-first MCP Registry descriptor for `callsieve mcp`; it never contacts the network or publishes automatically. Use `callsieve mcp-contract --out mcp-contract.json` to export the stable `callsieve_context` `structuredContent` contract that agents consume before broad search.

See [docs/INSTALL.md](docs/INSTALL.md) for human install and client setup, [docs/AGENT_CLI.md](docs/AGENT_CLI.md) for AI CLI behavior, and [docs/MCP.md](docs/MCP.md) for MCP examples across supported clients.

## Feedback FAQ

**Does it need to be MCP?** No. MCP is one integration path. The same retrieval path is available through `callsieve agent-context`, JSON output, Markdown output, lifecycle hooks or plugins where clients support them, and repo-local shims.

**Why not just Markdown or CSV?** Markdown is now available for direct reading, and JSON remains the default because agents and tooling need nested fields for files, symbols, snippets, tests, scores, and trace policy. CSV loses too much structure for this workflow.

**Is this already solved by IDE indexes?** IDE indexes are useful, but they are usually tied to one editor and optimized for interactive humans. CallSieve is agent-facing, cross-tool, local-first, auditable, and tuned to produce a compact read-first packet before an agent spends tokens on broad search.

**Does it work for PHP?** Yes. PHP files are indexed with lightweight detection for functions, classes, interfaces, traits, enums, imports, includes, references, and related snippets. If `intelephense` is installed, `--lsp` can report PHP language-server availability too.

## Local-First Guarantees

- no cloud services
- no API keys
- no proprietary code leaves the machine
- no SaaS app, auth system, web dashboard, or vector DB in the current local-first product
- MIT-licensed local core, with paid pilots and support kept separate from code access

## License

CallSieve is licensed under the [MIT License](LICENSE).

## Retrieval Model

CallSieve is sparse attention for the codebase before the prompt exists.

```text
User question
  -> repo index
  -> top-k files, symbols, tests, and import neighbors
  -> compact snippets
  -> agent context
```

The current retrieval model uses deterministic ranking first:

- exact symbol match
- exact path or filename match
- exported symbol substring match
- local symbol substring match
- keyword overlap
- likely related tests
- direct import neighbors where available

Embeddings, Git history, editor-specific extensions, and durable cross-repo memory are later phases.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

On Windows, `cargo test` may fail with `Access is denied` when replacing `target\debug\callsieve.exe` if a previous CallSieve daemon, MCP server, or shim-launched process still holds the binary. Stop the repo daemon first:

```bash
cargo run -- daemon-stop .
```

If the binary is still locked, terminate the stale process and rerun tests:

```cmd
taskkill /IM callsieve.exe /F
cargo test
```

For verification while investigating a lock, use a separate target directory:

```bash
cargo test --target-dir <temp-dir>\callsieve-target-test
```
