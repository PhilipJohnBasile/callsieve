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
callsieve stats <path>
```

Example:

```bash
cargo run -- index .
cargo run -- query . "where is auth handled?"
```

## What The MVP Does

- walks a repository while respecting common ignore rules
- detects TypeScript, JavaScript, Python, and Rust source files
- extracts practical first-pass symbols and imports
- stores a local JSON index at `.callsieve/index.json`
- returns compact JSON for agent consumption
- ranks matches with deterministic, explainable scoring

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
```
