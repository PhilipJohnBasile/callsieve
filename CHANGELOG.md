# Changelog

## Unreleased

### Added

- Added a 30-issue natural-language benchmark slice (`benchmarks/public/manifest-nl.json`) with prompts stripped of file paths, symbol names, and code snippets, plus a checked-in `compare-nl.json` result that isolates semantic retrieval quality.
- Added the Claude Code `proof-sprint` command group for buyer-facing observed proof workflows with init, status, collect, run/resume, and finalize steps.

### Changed

- Changed optional local embedding caches to `embeds.bin` format v4 with capped body-bearing symbol chunks and matched-symbol surfacing for semantic recall candidates.
- Re-ran the public hybrid A/B reports after the chunk-level refresh; hybrid remains non-regressing but still flat versus lexical on both the 50-issue and natural-language benchmark slices.

## v0.2.2 - 2026-06-06

### Added

- Added git activity signals to the local index and context packets, including recent commits, author counts, modification time, and churn.
- Added `agent-context --error <file>` to parse stack traces and promote indexed files named by resolved frames.
- Wired optional local embeddings into retrieval behind the `embed` feature and runtime `--embeddings` opt-in.
- Added chunked `embeds.bin` format v3 with chunk-to-file owners, optional chunk symbols, and stale-cache invalidation.
- Added semantic recall injection and shared semantic scoring so hybrid retrieval computes the query embedding once per context request.
- Added resumable public benchmark runs with `bench-run --compare --resume` and a checked-in 50-issue compare result.
- Refreshed the 50-issue public A/B report on current `main`, including query-kind and grep aggregate fields.

### Changed

- Kept lexical retrieval as the default path while documenting the opt-in hybrid, git boost, and stack-trace workflows.
- Updated docs to state the current public hybrid result honestly: parity with lexical retrieval on the 50-issue benchmark, not a quality-lift claim.

### Fixed

- Restored the missing git and stack-trace modules required by the schema 8 index and error-context ranking paths.

## v0.2.1 - 2026-06-04

### Added

- Surfaced CODEOWNERS ownership hints in compact context outputs.
- Added session metrics and public benchmark support for proof-oriented retrieval checks.
- Added Codex hooks and hardened agent-context enforcement for context-first workflows.
- Added an optional `embed` feature scaffold without changing the default local deterministic retrieval path.
- Added the VS Code extension scaffold and compile gate.
- Added commercial pricing, positioning, and roadmap documentation.

### Changed

- Updated the README with a competitor comparison that positions CallSieve as local-first retrieval infrastructure for coding agents.

## v0.2.0 - 2026-06-03

### Added

- Added MCP/rules/skills/setup-template support for VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, and Warp.
- Added `callsieve mcp-registry-manifest [--out <server.json>]` to generate a local-first MCP Registry descriptor for `callsieve mcp` without publishing or contacting the network.
- Added setup and strict-enforcement tests for the new clients, including JSON-preserving VS Code, Junie, and Zed configuration behavior.

### Changed

- Strict setup for the new clients now requires generated setup files, a fresh index, daemon state, and local shims, but does not require lifecycle hooks.
- Improved deterministic retrieval around MCP docs, command-surface files, generic action tokens, and test companion promotion.
- Updated README, install, MCP, agent CLI, benchmark, dogfood, and pilot docs for the expanded client and registry support.

### Fixed

- Zed setup now preserves invalid or JSONC `.zed/settings.json` files and writes a reviewable fallback template instead of overwriting them.
- Generated shareable rules and guidelines avoid embedding local executable paths, while path-specific config files remain ignored.
