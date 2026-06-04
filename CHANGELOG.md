# Changelog

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
