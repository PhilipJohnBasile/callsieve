# Contributing to CallSieve

CallSieve is a local-first Rust CLI that helps AI coding agents retrieve compact, relevant codebase context before they spend tokens on broad search. Contributions should protect that focus.

## Project Direction

Good contributions usually improve one of these areas:

- Deterministic retrieval quality for symbols, files, imports, references, tests, call paths, or blast-radius hints.
- Compact agent-facing output.
- Local-first setup for Codex, Claude Code, GitHub Copilot, OpenCode, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains, Amp, Goose, Warp, Cline, Zoo, Roo, or generic MCP clients.
- Evidence quality for traces, pilots, proof reports, evidence packs, and enterprise proof reports.
- Clear docs that match the actual CLI.
- Focused tests for indexing, retrieval, evidence gates, schemas, and CLI behavior.

Avoid changes that move the project toward hosted SaaS, authentication, a web dashboard, cloud dependencies, vector database infrastructure, or API-key-required workflows unless maintainers have explicitly accepted that direction first.

## Development Setup

Prerequisites:

- Rust toolchain managed by `rust-toolchain.toml`.
- A local clone of this repository.

Useful commands:

```bash
cargo fmt
cargo test
cargo clippy --all-targets --all-features
```

For local manual checks:

```bash
cargo run -- index .
cargo run -- agent-context . "find where retrieval ranking is handled"
cargo run -- status .
```

## Before Opening a Pull Request

- Keep the change focused.
- Add or update tests when behavior changes.
- Update docs when commands, schemas, outputs, setup flows, or product claims change.
- Keep output compact by default.
- Keep proof and benchmark claims honest and reproducible.
- Do not include proprietary code, private traces, secrets, API keys, or customer data.
- Run `cargo fmt` and `cargo test` when practical.

## Pull Request Guidance

In the PR description, explain:

- What changed.
- Why it matters for local-first agent context.
- How you tested it.
- Any docs, schema, or compatibility impact.
- Any known risks or follow-up work.

Small, evidence-backed pull requests are easier to review than broad rewrites.

## Issues

Use issue templates when possible. Include enough detail for a maintainer to reproduce the behavior locally, including:

- CallSieve version or commit.
- Operating system.
- Command run.
- Expected behavior.
- Actual behavior.
- Minimal sample repository or redacted fixture, if needed.

Do not file public issues with exploit details, proprietary source code, private session traces, secrets, or customer data.
