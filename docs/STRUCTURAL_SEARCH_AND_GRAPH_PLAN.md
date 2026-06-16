# Plan: Structural Search + Graph-Centrality Ranking + Graph Traversal

A roadmap for three retrieval upgrades that build on CallSieve's existing tree-sitter index and
symbol/reference/import graph:

1. **Structural search fallback** — match code by AST shape (powered by ast-grep) when the lexical
   packet misses, instead of only text/`rg`.
2. **Graph-centrality ranking** — a PageRank-style importance signal over the symbol/reference
   graph (a standard, freely reimplementable algorithm).
3. **Graph traversal MCP surface** — let an agent walk callers/callees/definitions over the graph
   CallSieve already indexes.

All three stay deterministic, local, and zero-AI-token.

## Guardrails that must hold for every item

- Default build stays dependency-light (new heavy deps go behind a Cargo feature, mirroring
  `embed` / `tokenizers`; external binaries are shelled out only when present, mirroring `--run-rg`).
- Retrieval stays deterministic and explainable (`--why-debug` / `ScoreComponent` must still account
  for every point).
- Zero AI model tokens spent on retrieval; no default network or upload.
- No regression on the checked-in 100% domain/language-smoke benchmark slices. New ranking signals
  are opt-in or additive, like `--bm25` / `--embeddings` / `--git-boost`.

## Licensing note

- **ast-grep** is MIT. If we link it as a crate (`ast-grep-core` etc.), cargo preserves its license
  in distribution as it does for every dependency — normal, not promotional crediting. If we shell
  out to a user-installed `ast-grep` binary, nothing is bundled at all.
- **PageRank** is a classic published algorithm; reimplementing it carries no attribution obligation.

---

## Item 1 — Structural search fallback (ast-grep)

**Why:** CallSieve's `grep` command (`src/cli/mod.rs:5963`, `Command::Grep`) returns a CallSieve
context packet and then optionally shells out to ripgrep via `run_rg` only when `--run-rg` is set
(`src/cli/mod.rs:5989`). That fallback is purely text/regex. Many real "grep only if needed" misses
are *structural* — "match arms returning `Err`", "functions taking `&CodeIndex`", "impls of trait
X". ast-grep matches by AST pattern over the same tree-sitter grammars CallSieve already uses, so a
structural fallback finds shape-based code the lexical packet and `rg` both miss, without leaving the
deterministic/local/no-cloud lane.

**Current state (verified):**
- `Command::Grep` handler at `src/cli/mod.rs:5963` builds context, optionally runs `run_rg`.
- The `--run-rg` pattern (shell out to an external binary, return its output after the packet) is the
  template to follow.
- CallSieve already depends on `tree-sitter` + JS/TS/Python/Rust grammars; ast-grep is built on the
  same tree-sitter foundation.

**Approach decision (recommended: shell-out first, library optional):**
- **A. Shell out to `ast-grep`/`sg` when present** (recommended first cut). Add `--run-sg <pattern>`
  (or `--structural <pattern>`) to `grep`, mirroring `--run-rg`: zero new Rust deps, no build weight,
  graceful "ast-grep not installed" message, fully opt-in. Matches the existing fallback ergonomics.
- **B. Link `ast-grep-core` behind a `structural` Cargo feature** (optional enhancement). Always
  available when built with the feature; deterministic without requiring the user to install
  ast-grep; heavier dep tree, so gated and off by default.

**Steps:**
1. Add a `run_sg(path, pattern, lang)` helper next to `run_rg`, shelling to `ast-grep run -p
   <pattern>` with JSON output, parsed into the same match-shape `run_rg` returns.
2. Wire `--run-sg`/`--structural` into `Command::Grep` and into the grep shim/guard path so the
   structural fallback is auditable under the same context-first policy.
3. Surface it as an MCP option on a grep-style tool (or document it as a CLI-only fallback first).
4. (Optional, later) Add feature `structural = ["dep:ast-grep-core", ...]` and an in-process matcher
   so the fallback works without an external binary.

**Validation:** golden tests that a known structural pattern (e.g. Rust `match` arm returning `Err`)
returns the expected files on this repo; "not installed" path returns a clean, non-fatal message.

**Payoff:** the "grep only if needed" loop gets structurally smart; recovers shape-based misses that
lexical ranking and `rg` cannot. Lowest-risk borrow — additive, opt-in, same stack.

---

## Item 2 — Graph-centrality (PageRank) ranking signal

**Why:** CallSieve already has a symbol/import/reference graph and a `add_graph_consensus_boost`
(`src/query/mod.rs`) that boosts neighbors of top-ranked anchors for natural-language queries. But it
has no notion of *global* importance — a file that the whole codebase depends on (high centrality)
isn't preferred over a leaf file with the same lexical score. Aider's repo-map ranks symbols by
PageRank over the tag graph for exactly this reason. Adding a centrality signal is deterministic and
complements the in-flight BM25+ work (BM25+ scores query relevance; PageRank scores structural
importance).

**Current state (verified):**
- Graph adjacency already available via `IndexLookup`: `references_from_path`, `references_to_path`,
  `imports_to_path`, `resolved_imports_for_file` (used inside `add_graph_consensus_boost`).
- Graph records: `ReferenceRecord` (`src/store/mod.rs:123`), `ImportRecord` (`src/store/mod.rs:102`).
- Ranking pipeline: `ContextCandidate` (`src/query/mod.rs:1425`), `score()` (`:1456`), then layered
  boosts (`apply_git_boost`, `add_graph_consensus_boost`, `sort_candidates_lexical`,
  `apply_hybrid_ranking`).

**Steps:**
1. At index time, build a file-level directed graph (nodes = files; edges = imports + references) and
   compute PageRank with standard damping (d = 0.85) to a fixed iteration count / convergence epsilon
   — deterministic. Store a per-file centrality score on the index (additive field, `#[serde(default)]`,
   so old indexes still load; a fresh index — already auto-rebuilt when stale — populates it).
2. Add a centrality ranking signal as a `ScoreComponent` (`src/query/ranker.rs`) so `--why-debug`
   explains it (e.g. `central: pagerank=0.0142`).
3. Gate it like the other ordering signals: opt-in global `--pagerank` (matching `--bm25`/`--git-boost`)
   so default ranking and the 100% gates stay byte-identical until measured.
4. Normalize centrality into the existing point scale (it is a tie-breaker / modest boost, not a
   dominant term) so a central-but-irrelevant file never outranks a relevant leaf.

**Validation:** unit test PageRank on a tiny fixture graph (hub scores > leaf). Before/after on
`eval-retrieval`, the NL manifest, and language-smoke slices; gate on no regression to the 100%
domain slices. Measure whether centrality lifts NL first-correct-file@5 (same harness as BM25+).

**Payoff:** structural-importance signal that helps vocabulary-gap NL queries land on the file the
codebase actually centers on; deterministic; pairs with BM25+.

---

## Item 3 — Graph traversal MCP surface

**Why:** CallSieve already exposes `callsieve_focus` (calls / called_by for a symbol) and
`callsieve_related` (imports, callers, callees, blast radius for a file). What it lacks is a
*multi-hop walk* — "from this symbol, show callers two hops out", "what defines the type returned
here". The recent tree-sitter knowledge-graph / Codebase-Memory MCP pattern is exactly this:
traversable graph exploration over MCP. It's incremental for CallSieve because the edges already
exist in the index.

**Current state (verified):**
- MCP tools `callsieve_focus`, `callsieve_related`, `callsieve_tests` already traverse one hop.
- Edge data and adjacency helpers already exist (see Item 2).

**Steps:**
1. Add an MCP tool `callsieve_graph_neighbors` (and matching CLI) taking `path`, a `file` or
   `symbol` anchor, a `direction` (`callers` | `callees` | `imports` | `imported_by` | `both`), and a
   bounded `depth` (default 1, hard cap small, e.g. 3) over the existing adjacency helpers.
2. Return a compact, deterministic adjacency packet (anchor + neighbor edges with file/symbol/line),
   reusing the `FocusEdge`/`ReferenceEdge` shapes so output stays consistent and token-light.
3. Enforce a node/edge cap (like `MAX_FOCUS_GRAPH_EDGES`) so deep walks can't blow the token budget.

**Validation:** unit test multi-hop traversal on a fixture (depth-2 callers); MCP smoke test the new
tool; confirm output respects the edge cap.

**Payoff:** lets agents explore blast radius beyond one hop without full-file reads — a richer
traversal surface on data CallSieve already indexes. Additive and optional.

---

## Sequencing

1. **Item 1 (ast-grep fallback)** — additive, opt-in, lowest risk; immediate value on shape-based
   misses. Ship the shell-out version first.
2. **Item 2 (PageRank)** — highest retrieval-quality upside; do it alongside/after BM25+ so both are
   measured on the same NL harness. Opt-in until proven.
3. **Item 3 (graph traversal MCP)** — incremental surface on existing data; do when there's demand
   for multi-hop exploration.

## Out of scope

- SCIP/LSIF (Sourcegraph) cross-repo code-intelligence indexers — powerful but heavy infra against
  the "slimmest architecture, no cloud" wedge; CallSieve's optional LSP enrichment already covers the
  precise-reference need.
- ast-grep's lint/rewrite engine — CallSieve is a retrieval layer, not a code-modification tool;
  only ast-grep's *search* belongs here.
