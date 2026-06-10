# Changelog

## Unreleased

### Changed

- Bench-validated the ranking changes shipped after v0.3.2: identifier-kind queries keep pure lexical order in hybrid retrieval (semantic reordering of them produced zero wins and one persistent loss across every configuration), semantic similarity cannot lift a test file above source it trailed lexically, and test-companion eviction tie-breaks on original rank instead of current position. Result: the 50-issue SWE-bench Lite set now scores `60.0%` lexical = `60.0%` hybrid (50 ties, zero losses) and the natural-language slice keeps `23.3%` hybrid vs `20.0%` lexical — `--embeddings` is strictly non-regressing, resolving the hybrid edge recorded in v0.3.2.

- Daemon serving skips the per-request freshness stat walk while the poll loop's own verification is recent (within 2× the poll interval), removing per-file stat calls from the hot path; an unverified or stalled loop falls back to the full check. The staleness window is unchanged from what the daemon's refresh interval already promises.

## v0.3.2 - 2026-06-09

### Added

- Added `--embed-model <small|code>` to `index` and `agent-context` (embed builds): `code` selects jina-embeddings-v2-base-code, a code-tuned 768-dim model trained on docstring-to-code pairs, as an opt-in quality tier. The default stays BGE-small. On the 30-issue natural-language SWE-bench slice the code model lifts hybrid first-correct-file@5 to `26.7%` vs `20.0%` lexical (+6.7 pp, 2 wins, 0 losses) — double the BGE-small reranking lift (+3.3 pp) — through better reranking; union-pass injection still does not fire. It is ~4× slower to embed than BGE-small, so it stays opt-in. The embeds cache is keyed by model id, so switching models rebuilds it automatically; benchmarks select the model via `CALLSIEVE_BENCH_EMBED_MODEL=code`.
- The daemon now holds the parsed index in memory and serves `agent-context` over a local Unix socket (`.callsieve/daemon.sock`); the CLI tries the daemon first and silently falls back to direct loading (`--no-daemon` forces direct). Output is byte-identical, and a per-request stat-level freshness check refuses stale serves. On a 2.7k-file Django checkout `agent-context` drops from 0.61s to 0.31s. Non-unix targets keep the direct path.
- Added `callsieve index-export <repo> --out <file>` and `callsieve index-import <repo> --from <file> [--allow-partial]` for team warm starts without any cloud: one machine exports its index, another verifies every file by content hash (not mtime, which never matches across machines), rewrites local file stats so freshness checks pass, and skips the full re-index. Verified on a 3.3k-file Django checkout: import matches all files and `status` reports fresh.
- Added `callsieve setup-auto <repo> [--force] [--dry-run]`: detects installed agents on this machine (binary on PATH, config directory — including Linux `~/.config` locations — macOS app bundle, or VS Code extension) and runs the existing per-client setup for each, with no per-client decisions. Hook-capable clients (Codex, Claude Code, Copilot, OpenCode, Antigravity, Cline) also get non-strict lifecycle hooks installed, since hooks are the strongest context-first integration.
- Claude Code and generic client Stop hooks now report a factual session summary when CallSieve served context: packets, packet tokens, read-first files, and the zero-model-token retrieval cost. Estimated savings claims stay gated behind audited observed-session reports.

### Changed

- The daemon no longer rebuilds the index on every poll tick: with the in-memory copy it stat-checks freshness (~ms) and rebuilds only when files actually changed. On the Django checkout this takes the idle daemon from ~100% CPU (continuous re-index) to ~0.4%.
- The MCP server caches the parsed index in-process and revalidates freshness per call, so repeat tool calls skip the index parse entirely.
- Filename-stem ranking matches now scale with corpus rarity: a query token naming a unique file (e.g. `sqlmigrate`) gets a decisive boost, while a stem that is also an everyday corpus word (e.g. `schema`, which several files are named after) keeps roughly its old weight. Public 50-issue SWE-bench Lite first-correct-file@5: lexical `56.0%` → `60.0%` (+4.0 pp, 2 fixes, 0 regressions), hybrid `56.0%` → `58.0%`; the 30-issue natural-language slice is unchanged. Known hybrid edge recorded by the run: semantic reranking can demote a rank-5 lexical hit (astropy-14182).
- Index schema 9: `index.json` is now compact (not pretty-printed) and `ReferenceRecord` serialization drops derivable fields (`file_id`, `confidence`, redundant `source_range`, null targets, default `kind`/`edge_source`). On a 2.7k-file Django checkout the index shrinks from 247 MB to 111 MB (−55%) with identical query output, `status` drops from 0.44s to 0.30s, and `agent-context` from 0.77s to 0.61s; older indexes remain readable and rebuild automatically.

- Query-time ranking is faster: `TokenWeights` no longer materialises a full term set per indexed file (cloning content terms and tokenizing every symbol) on each query; a substring prefilter skips symbols that cannot match. Roughly 6-10% faster `agent-context` on a 2.7k-file repo with byte-identical output.

## v0.3.1 - 2026-06-09

### Changed

- Raised `MAX_SYMBOL_CHUNKS_PER_FILE` from 2 to 8: the embedder now indexes up to 8 top-level symbols per file (largest first by line span) instead of 2, giving the semantic ranker broader symbol coverage of large files.
- Lowered the `NaturalLanguage` cosine floor from 0.15 to 0.10 (`Identifier` floor stays at 0.25) to let the semantic component contribute when vocabulary-gap cosines are low.
- Bumped `embeds.bin` to format v5; existing caches rebuild automatically on next run.

### Benchmarks

- Public hybrid A/B re-run (June 2026, chunk-cap-8 + NL-floor-0.10): `56.0%` lexical = `56.0%` hybrid (`+0.0 pp`, 50 ties) on the 50-issue SWE-bench Lite set; `20.0%` lexical → `23.3%` hybrid (`+3.3 pp`, 1 win, 0 losses, 29 ties) on the 30-issue natural-language slice. The NL lift comes from semantic reranking within the existing 8-candidate lexical pool — no union-pass injection fired. BGE-small cosines for vocabulary-gap misses stay below 0.10 even with 4× more indexed symbols per file; a stronger embedding model is the next lever.

## v0.3.0 - 2026-06-08

### Added

- Added `callsieve focus --line <n>` for same-name symbol disambiguation; generated expansion commands include `--line` so agents always land on the right symbol.
- Added `callsieve focus --references` to opt in to non-call reference listing (off by default to keep focus output compact).
- Added competitive-positioning ranking boost: docs files matching competitor or positioning intent tokens (aider, cursor, copilot, cody, devin, greptile, windsurf, etc.) rank above generic setup docs on product-strategy tasks.
- Added `docs/COMPETITIVE.md` with a competitor table, product priorities, and do-not-chase constraints.
- Added query-kind-aware cosine floor to the semantic union pass: `NaturalLanguage` queries use floor 0.15 and `Identifier` queries use floor 0.25, replacing the single hard-coded 0.30. The architecture is ready to benefit from a stronger embedding model.
- Added `FocusTarget` struct and `context_read_first_targets()` to the public query API for typed expansion targets.
- Added `test/` and `src/test/` as recognised test directory patterns alongside `tests/` and `src/tests/`.

### Changed

- `agent-context` skim packet is now fully compact: indexed `instruction.x.o/n` targets (read-first array offsets instead of duplicated file paths), short signal codes (`sym`, `sy`, `kw`, `ct`, `pt`, `test`), positional `i` impact arrays, `g.u`/`g.d` graph previews on the top file only, and dropped duplicate `kw` reasons when `sy` already covers them.
- Token budget enforcement now applies to the full agent-facing response envelope: compacts task-memory hints, trims expansion/graph/call-path fields, shrinks metadata, and drops files only as a last resort.
- MCP `callsieve_context` now emits structured tool-call instruction blocks for `o`/`r`/`t` expansion targets, mirroring the CLI skim budget.
- VS Code extension now renders graph hints (`g.u`/`g.d` upstream/downstream previews), compact impact arrays, selection summary, and structured MCP instruction blocks; indexed `instruction.x` targets replace legacy path-based targets.
- VS Code extension default limit changed from 8 to 5 to match `DEFAULT_AGENT_CONTEXT_LIMIT`.
- `embeds.bin` format v4 with capped body-bearing symbol chunks and matched-symbol surfacing for semantic recall candidates.

### Fixed

- Fixed the non-embed stub of `add_semantic_candidates` to accept the `query_tokens` parameter added for per-kind cosine floors.

### Benchmarks

- Public hybrid A/B re-run (June 2026, lowfloor): `56.0%` lexical = `56.0%` hybrid (`+0.0 pp`, 50 ties) on the 50-issue SWE-bench Lite set; `20.0%` lexical = `20.0%` hybrid (`+0.0 pp`, 1 win, 1 loss, 28 ties) on the 30-issue natural-language slice. Union-pass injection confirmed zero even at the new lower floors — BGE-small cosine scores for vocabulary-gap misses stay below threshold. Both slices remain `+50.0 pp` and `+6.7 pp` above naive grep respectively.

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
