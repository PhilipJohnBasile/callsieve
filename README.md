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
callsieve index <path>
callsieve symbols <path>
callsieve symbol <path> <symbol_name>
callsieve query <path> "<question>"
callsieve context <path> "<task>" [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve benchmark <path> "<task>" [--limit <n>] [--snippets-per-file <n>] [--no-snippets]
callsieve stats <path>
```

Example:

```bash
cargo run -- index .
cargo run -- query . "where is auth handled?"
cargo run -- context . "change login token expiry behavior"
cargo run -- benchmark . "change login token expiry behavior"
```

## What The MVP Does

- walks a repository while respecting common ignore rules
- detects TypeScript, JavaScript, Python, and Rust source files
- extracts practical first-pass symbols and imports
- stores a local JSON index at `.callsieve/index.json`
- returns compact JSON for agent consumption
- ranks matches with deterministic, explainable scoring
- builds compact read-first context packets for coding tasks
- estimates context-packet token savings versus a naive grep/read loop

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
        "risk": "medium"
      },
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

## Local-First Guarantees

- no cloud services
- no API keys
- no proprietary code leaves the machine
- no SaaS app, auth system, web dashboard, vector DB, or MCP server in the MVP

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

Embeddings, richer call graphs, MCP, LSP, Git history, and long-term memory are later phases.

## Development

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```
