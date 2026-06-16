# Plan: Retrieval & Token-Efficiency Improvements

A roadmap for three retrieval and token-accounting upgrades built from standard, publicly
documented techniques — exact BPE token counting (OpenAI's tiktoken encodings), code
skeletonization (a repo-map / signature-elision technique), and BM25+ ranking (Robertson/Sparck
Jones term weighting with length normalization). Each is implemented from scratch as part of
CallSieve and tuned to CallSieve's architecture.

This document is the implementation plan. TOON re-encoding and SimHash dedup are deliberately out of
scope (diminishing returns against CallSieve's already-extreme JSON compaction).

> **Status (implemented on branch `feat/retrieval-improvements`):** All three items below are
> implemented and tested. Notes on what shipped vs. the original sketch:
> - **Item 1** ships as the `tokenizers` Cargo feature + global `--tokenizer` flag (default
>   `heuristic` is byte-identical to the old `bytes/4`). The trim loops already re-measure with
>   the active counter every iteration and only ever remove data, so no separate "revert" step was
>   needed.
> - **Item 2** ships as `focus --skeleton` (+ MCP `skeleton` arg), reusing tree-sitter-derived
>   symbol line ranges via a deterministic structural skeletonizer.
> - **Item 3** ships as the opt-in global `--bm25` flag applying BM25+ **length normalization** to
>   the content-keyword component. Term frequency is binary because the index stores a deduplicated
>   content-term *set*, not raw counts; adding true per-term counts is a follow-up that needs an
>   index schema change. Default ranking is unchanged (unit-tested equivalence), so the 100%
>   benchmark gates hold. Local gates checked: `eval-retrieval` and the public requests seed stay at
>   their baselines with `--bm25`; the full public NL/Django sweep is a longer follow-up run.

## Guardrails that must hold for every item

- Default build stays dependency-light (new deps go behind a Cargo feature, mirroring `embed`).
- Retrieval stays deterministic and explainable (`--why-debug` / `ScoreComponent` must still account
  for every point).
- Zero AI model tokens spent on retrieval; no default network or upload.
- No regression on the checked-in 100% domain/language-smoke benchmark slices.

---

## Item 1 — Real tokenizer (replace the bytes/4 estimate)

**Why:** CallSieve's entire pitch is token savings, but budget enforcement and every `benchmark` /
`context_payload_reduction` / `proof-report` number run on an estimate, not real counts. A skeptical
enterprise buyer can attack the estimator. Counting with the provider's real tokenizer makes budget
enforcement accurate per model and closes that hole.

**Current state (verified):**
- `estimate_tokens` at `src/query/mod.rs` is `text.len().div_ceil(4)`.
- JSON wrapper `value_estimated_tokens`.
- ~30 call sites across `src/mcp.rs`, `src/cli/mod.rs`, `src/query/mod.rs`, all routing through those
  two functions. Two jobs: budget trim loops and proof/savings math.

**Steps:**
1. `Cargo.toml`: add a gated feature mirroring `embed`, e.g. `tokenizers = ["dep:tiktoken-rs"]`.
   Default build unchanged.
2. New `src/query/tokens.rs` with a `TokenCounter` enum:
   - `Heuristic` → current `len()/4` (default, zero-dep, ships today).
   - `Tiktoken(o200k)` / `Tiktoken(cl100k)` → behind the feature flag.
   Determinism preserved: same input → same count, just a more accurate deterministic function.
3. Route `estimate_tokens` / `value_estimated_tokens` through the active `TokenCounter`. Single
   choke-point swap — no call-site churn.
4. Thread `--tokenizer heuristic|o200k|cl100k` (with an `agent-context` / MCP default) so trim loops
   and proof math use the same counter.
5. Trim loops re-measure with the active counter each iteration and only ever remove data, so they
   inherit accurate enforcement directly.

**Validation:** existing tests pass with `Heuristic` (byte-identical behavior). New tests assert the
feature-built counter on known fixtures. Regenerate proof reports and confirm they still gate.

**Payoff:** tokenizer-accurate proof artifacts; biggest strengthening of the proof posture.

---

## Item 2 — Code skeletonization (signatures, bodies elided)

**Why:** `focus` returns up to 120-line bounded snippets; packets list symbols. A skeleton view
(signatures + type decls + doc lines, bodies collapsed) shows *more* relevant code at *lower* token
cost — directly serving the compact-packet promise.

**Current state (verified):** tree-sitter is already a dependency (`tree-sitter`, plus JS/TS/Python/
Rust grammars in `Cargo.toml`); parses live in `src/indexer/tree_sitter_symbols.rs`. No new deps for
the four proven languages.

**Steps:**
1. New `src/query/skeleton.rs`. Use the existing tree-sitter-derived symbol line ranges; keep
   signatures, type declarations, and leading doc comments; replace function/method bodies with a
   `{ … }` marker plus an `omitted_lines` count (reuse the shape `focus` already returns for
   truncation).
2. Surface as `callsieve focus … --skeleton` and/or a profile tier between `skim` and `normal`.
3. Graceful fallback: heuristic-only (non-tree-sitter) languages return the existing bounded snippet.

**Validation:** golden-file tests for skeleton output per language; confirm token count drops vs full
snippet using Item 1's counter.

**Payoff:** more signal per token; lowest risk (additive, opt-in).

---

## Item 3 — BM25+ ranking

**Why:** lift the weak natural-language slice (README: 36.7% hybrid-compare vs 100% domain-aliased)
while staying deterministic and embedding-free.

**Current state (verified):** `src/query/ranker.rs` already has IDF-style rarity weighting
(`idf_weight`, clamped `[0.2, 1.0]`) and `overlap_points`, and document frequency per query token is
computed in `TokenWeights::new`. But scoring is **membership-based additive points** — no
term-frequency saturation, no document-length normalization. The `[0.2, 1.0]` clamp is already
morally the BM25+ δ floor.

**Steps:**
1. At index time add cheap aggregates: average document length (and, in a follow-up, per-document
   term frequency).
2. Replace `overlap_points` for the content-keyword component with a BM25+ term score:
   `idf × ((tf·(k1+1)) / (tf + k1·(1 − b + b·dl/avgdl)) + δ)`. Reuse the existing IDF clamp as δ.
3. Keep it inside the `ScoreComponent` system so `--why-debug` still explains points.
4. Validate before/after with `eval-retrieval`, the NL manifest, and language-smoke slices; gate on
   no regression to the 100% domain slices.

**Payoff:** distinguishes "matched once" from "matched many times" and normalizes for file length —
the change most likely to move first-correct-file@5 on NL queries.

---

## Sequencing

1. **Item 1 (tokenizer)** — contained feature-flagged swap; makes Items 2 and 3 measurable.
2. **Item 2 (skeletonization)** — additive, opt-in, immediate packet wins.
3. **Item 3 (BM25+)** — highest upside, most validation; do last when it can be measured accurately.

## Out of scope

- TOON / record-array re-encoding — diminishing returns vs current compaction.
- SimHash near-duplicate detection — niche; revisit only if generated/vendored duplicates show up in
  packets.
- A request-path proxy / log-template folding / output-control rewriting — these serve a runtime-
  interception model. CallSieve is deliberately *not* in the request path (retrieval happens before
  the prompt exists), so adopting them would break the "slimmest architecture" wedge.
