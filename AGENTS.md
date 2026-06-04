# CallSieve Agent Instructions

## Product

CallSieve is a local-first codebase intelligence layer for AI coding agents.

The core pain:
Developers are wasting huge percentages of agent context windows on grep, ripgrep, file discovery, repeated file reads, and re-discovering codebase structure.

The product promise:
Stop paying AI agents to grep your repo.

The token promise:
CallSieve spends zero AI model tokens on retrieval by ranking against a local index. The returned context packet still consumes agent context tokens when read, so keep it compact by default.

The product should give coding agents precise, compact, structured answers about:
- symbols
- files
- modules
- imports
- call paths
- references
- tests
- ownership
- risk / blast radius
- relevant snippets

CallSieve is not "another coding agent." It is the context and retrieval layer underneath coding agents.

## Positioning

Tagline:
The codebase filter for AI coding agents.

Longer pitch:
CallSieve gives Codex, Claude, Copilot, OpenCode, Antigravity, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains, Amp, Goose, Warp, Cline, Zoo, Roo, and generic local coding agents the exact symbols, call paths, files, tests, and snippets they need without wasting tokens on grep.

Positioning pillars:
- Slimmest architecture: Rust CLI, local JSON index, deterministic ranking, no cloud service, no API key, no vector DB, and no web dashboard.
- Best agent-agnostic setup story: Codex, Claude, Copilot, OpenCode, Antigravity, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains, Amp, Goose, Warp, Cline, Zoo, Roo, and generic MCP.
- Strongest proof posture: benchmark, eval, trace, pilot, proof-report, enterprise-proof-report, and evidence-pack.
- Strongest token-saving positioning: read these files first, grep only if needed.

## Current Product Scope

Keep CallSieve a local CLI-first Rust project. The current implementation already has indexing, retrieval, MCP tools, context-first enforcement, daemon/watch freshness, observed-session evidence collection, proof reports, and enterprise-proof gates.

Core user-facing commands:

```bash
callsieve index <path> [--lsp]
callsieve agent-context <path> "<task>"
callsieve context <path> "<task>"
callsieve query <path> "<natural language question>"
callsieve symbol <path> <symbol_name>
callsieve demo <path> [--task "<task>"]
callsieve memory-clear <path>
callsieve status <path>
callsieve mcp
callsieve mcp-config <path> [--format json|toml]
callsieve proof-report <manifest.json>
callsieve enterprise-proof-report <manifest.json>
```

The product should continue to:

- index a repository locally
- parse source files
- extract symbols
- extract imports
- extract references, calls, related tests, and blast-radius hints
- store an index locally
- return compact JSON or human-readable output
- enforce a context-first workflow before broad grep where possible
- keep observed proof separate from controlled replay
- favor useful results over perfect static analysis
- avoid requiring cloud services
- avoid requiring API keys
- avoid sending proprietary code anywhere

## First Language Targets

Start with:

- TypeScript / JavaScript
- Python
- Rust

Use a clean architecture so more languages can be added later.

## Preferred Technical Direction

Use Rust.

Likely crates:

- clap for CLI
- serde / serde_json for output
- anyhow / thiserror for errors
- ignore or walkdir for file walking
- tree-sitter for parsing where useful
- tracing for logs

Do not overcomplicate the first version.

Start with a working local JSON index and simple retrieval before building SQLite, MCP, or embeddings.

## Architecture

Suggested crates/modules:

```text
callsieve
  src/
    main.rs
    cli.rs
    indexer/
      mod.rs
      walker.rs
      language.rs
      symbols.rs
      imports.rs
    store/
      mod.rs
      json_store.rs
    query/
      mod.rs
      ranker.rs
      formatter.rs
    output/
      mod.rs
      json.rs
      text.rs
```

## Retrieval Architecture

CallSieve should behave like sparse attention for a codebase before the prompt exists.

The core retrieval flow:

```text
User question
  -> cheap index scoring
  -> top-k files, modules, symbols, and tests
  -> focused snippet extraction
  -> compact agent context
```

The product solves upstream selection. Sparse attention models solve downstream computation.
If the right code never reaches the prompt, sparse attention cannot recover it.
CallSieve should make sure the right code is selected before an agent spends tokens reasoning.
Expose `retrieval_cost.retrieval_model_tokens = 0` in context-bearing outputs, and keep context packet token estimates separate from observed whole-session token savings.

Prefer semantic code units over arbitrary token blocks:

- symbol spans
- classes and modules
- exported functions
- test files
- routes and handlers
- config files
- dependency clusters

Start with deterministic ranking:

- exact symbol and path matches
- filename and directory matches
- import graph proximity
- test-name proximity
- keyword overlap

Embeddings and rerankers can come later, after the CLI proves the core retrieval loop.

Later phases:

- richer call graph
- semantic embeddings
- Git history graph
- agent memory cache
- deeper editor integrations
- wider language ecosystem coverage

## Design Principles

Token savings matter.
Every output should be compact by default.

Local-first.
The user owns the code. No code leaves the machine.

Agent-friendly.
Outputs should be useful to LLMs, not only humans.

Grep is the baseline enemy.
Every feature should reduce the need for blind grep/read loops.

Progressive enhancement.
Start with symbols and paths. Add call graph and semantic retrieval later.

Fast over fancy.
A fast 80% useful index is better than a perfect academic analyzer.

## Output Style

Prefer compact outputs like:

```json
{
  "query": "where is auth handled?",
  "matches": [
    {
      "file": "src/auth/session.ts",
      "symbol": "createSession",
      "kind": "function",
      "score": 0.91,
      "lines": [12, 48],
      "why": "session creation and auth token handling"
    }
  ]
}
```

## Engineering Standards

Write clear Rust.
Keep modules small.
Add tests for parsing/indexing behavior.
Prefer simple deterministic ranking before embeddings.
Use cargo fmt.
Use cargo clippy when practical.
Use cargo test before calling work complete.
Do not use em dashes in user-facing copy.
Do not operate in Dropbox or Dropbox-synced paths.

## Current Goal

Harden the local-first agent-context and proof workflow:

- keep deterministic retrieval compact and explainable
- improve references, call paths, tests, and blast-radius quality
- keep MCP and agent setup aligned with the CLI
- keep trace, pilot, proof-report, evidence-pack, and enterprise-proof-report schemas honest
- add tests for evidence gates and retrieval behavior
- keep README, product brief, and docs aligned with the actual app

Do not start with a SaaS app.
Do not start with auth.
Do not start with a web dashboard.
Do not start with vector DB infrastructure.
Do not add vector DB infrastructure or cloud dependencies until the local CLI proves the core value.
