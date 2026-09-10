# Product background and evaluation notes

These are the detailed positioning and evaluation notes moved out of the README on September 10, 2026. Benchmark percentages apply only to the named dataset and configuration. For the shortest path to using CallSieve, start with the [README](../README.md).

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

CallSieve's core local engine is open source under the [MIT License](../LICENSE).

The public repo includes the local CLI, MCP server, repository indexer, deterministic retrieval, agent-context workflow, benchmark harness, proof reports, docs, and tests. The commercial motion is not selling access to the code. It is selling outcomes around it: paid pilots, local installation, agent integration, evidence collection, retrieval tuning, private workflow support, and enterprise proof reporting.

For proposed commercial packages and placeholder pricing, see [commercial/PRICING.md](../commercial/PRICING.md).

Keep broad claims gated. Use `context_payload_reduction` for estimated prompt-payload savings, and use observed token reduction only when real paired transcripts provide audited token counts.

## Current State

CallSieve is now an open-source local Rust CLI with a JSON index, deterministic retrieval, optional local embeddings, optional LSP reference enrichment, CODEOWNERS and git-history signals, stack-trace-aware error context, lifecycle hooks for Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, and Cline, MCP/rule/template setup for Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, and Zoo Code, context-first guardrails, a daemon that keeps the index fresh and serves agent-context from memory over a local socket (0.31s vs 0.61s direct on a 2.7k-file repo, byte-identical output), team index warm starts via `index-export`/`index-import`, one-command `setup-auto` agent detection, benchmark reports, observed-session traces, and gated proof reports. `public-proof-report` now exposes 19/19 default-layer client setup coverage so the agent-neutral MCP story is checked in the same artifact as retrieval proof.

Language coverage is now broad enough for common multi-language repos: Python, TypeScript, JavaScript, C++, Java, C#, Go, Rust, SQL, Kotlin, Swift, Dart, PHP, Ruby, Bash, PowerShell, C, Scala, Elixir, Lua, Objective-C, Zig, Julia, OCaml, Haskell, and PL/SQL are indexed with lightweight symbol extraction and practical import/include detection. Tree-sitter still provides richer parsing for the proven first languages, while the long tail uses deterministic local heuristics that can improve without changing the zero-cloud architecture.

Public retrieval result (June 2026): deterministic Mode A first-correct-file@5 is `100.0%` on the 50-issue SWE-bench Lite subset in `benchmarks/public/manifest-50.json` (`benchmarks/public/results/mode-a-50-domain.json`), with zero remaining public misses in that checked slice. A third-repo 10-task `psf/requests` seed in `benchmarks/public/manifest.json` also passes at `100.0%` (`benchmarks/public/results/mode-a-requests-seed.json`), bringing the public proof gate to 186 evaluated public tasks across strict compare, deterministic, natural-language, and language-smoke reports. The stricter checked-in compare gate still proves `60.0%` lexical first-correct-file@5 with a `+54.0 pp` lift over the naive-grep baseline of `6.0%`, and an opt-in hybrid comparison reaches `60.0%` with 50 ties and zero losses. On the 30-issue natural-language slice in `benchmarks/public/manifest-nl.json`, deterministic Mode A now reaches `100.0%` first-correct-file@5 (`benchmarks/public/results/mode-a-nl-domain.json`) and the public proof target is `100.0%`; the stricter compare report still records `36.7%` hybrid with `+23.3 pp` over grep. Public pinned language-smoke slices also pass at `100.0%` first-correct-file@5 for Rust (`benchmarks/public/results/mode-a-rust-callsieve.json`) and TypeScript (`benchmarks/public/results/mode-a-typescript-callsieve.json`). Full-repo prompt-pack proxy baselines measure Astropy at `5,638,468` estimated tokens and Django at `5,155,623` estimated tokens, both over `1,000x` the default CallSieve proof packet; the smaller Requests checkout measures `101,186` estimated tokens, still over `20x` the compact packet. The mechanism came from checked-in measurements: Python settings/constants are indexed as symbols, common NL code vocabulary is bridged to path/symbol terms, and framework-domain module aliases connect human task language to implementation modules such as WCS, migrations, SQL compilers, SQL query builders, ORM lookups, autoreloaders, serializers, validators, enums, URL resolvers, SQLite test-database creation, SQL order-by relation compiler routing, SQL query-builder routing for filterability and combined-query issues, auth proxy-permission migration routing, Requests session method-normalization routing, Requests dependency-exception pass-through routing, `UniqueConstraint` model field-check routing, `ForeignKey` `to_field` rename autodetection, and dynamic `SCRIPT_NAME` static/media settings resolution.

For human installation and client setup, see [docs/INSTALL.md](../docs/INSTALL.md). For AI CLI and wrapper behavior, see [docs/AGENT_CLI.md](../docs/AGENT_CLI.md). For observed whole-session proof collection, see [docs/OBSERVED_SESSIONS.md](../docs/OBSERVED_SESSIONS.md). For dogfooding and less-grep measurement, see [docs/DOGFOOD.md](../docs/DOGFOOD.md). For paid pilot packaging, see [docs/PILOTS.md](../docs/PILOTS.md). For competitive positioning and product gaps, see [docs/COMPETITIVE.md](../docs/COMPETITIVE.md).

The core workflow is:

```text
index repo -> ask for agent context -> read returned files first -> grep only if needed -> audit traces and savings
```
