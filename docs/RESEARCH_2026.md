# Code Retrieval — June 2026 SOTA, mapped to CallSieve

A scan of the June-2026 code-retrieval / agentic-context literature, mapped onto CallSieve's existing
[ROADMAP](../ROADMAP.md). **Conclusion: CallSieve's bets are SOTA-aligned** — deterministic AST index,
zero-cloud, hybrid-with-optional-embeddings, and `first_correct_file_rate` as the headline metric are all
exactly where the 2026 frontier landed. The research adds **three concrete upgrades that fit the existing
workstreams** without touching the determinism guarantee.

## Where the field is (June 2026)

- **Graph-RAG over code is the frontier.** RepoGraph (repo-level code graphs → **+32.8% relative on SWE-bench**),
  CodexGraph (graph-DB interface for agents), GraphCoder, and Codebase-Memory (tree-sitter knowledge graphs served
  via MCP). *"Reliable Graph-RAG for Codebases"* benchmarks deterministic AST-graphs vs LLM-extracted KGs vs
  vector-only and finds **deterministic AST graphs are competitive and far cheaper to build.** CallSieve already
  extracts symbols + calls/refs/imports via tree-sitter — the code-context graph is the natural next index, and it
  stays zero-cloud.
- **Hybrid + late-interaction is the retrieval SOTA.** Production retrieval is BM25/lexical + dense, merged, then
  **reranked with ColBERT-style MaxSim late-interaction** (ColBERT / ColPali / SPLADE). **Wholembed-v3** (Mar 12 2026)
  is a unified late-interaction model for *code* with AST-aware parsing and a two-stage prune→MaxSim engine (~50 ms P50).
- **Localization agents** (LocAgent, CoSIL, GraphLocator, OrcaLoca, and "One Tool Is Enough" RL-trained repo
  navigation) model the repo as a graph and prune context aggressively. CallSieve's `agent-context` is the
  **deterministic, zero-model-token** version of the same idea.
- **Agentic Context Engineering (ACE)** treats context as an evolving playbook (generate → reflect → curate) and
  warns of a *brevity bias*. CallSieve's compact packets + traces are the auditable instance.
- **SWE-bench is saturating** — OpenAI stopped reporting SWE-bench Verified in early 2026 because "scoring well" and
  "being useful" diverged. The field is moving to real-task benches and first-correct-file. **CallSieve already leads
  with `first_correct_file_rate` — ahead of the curve.**

## Concrete upgrades (fit the existing ROADMAP)

1. **Workstream 2 (hybrid retrieval) → add late-interaction reranking.** Keep the deterministic floor + opt-in
   embeddings exactly as planned, but rerank the top-K with a **ColBERT-style MaxSim** over local token embeddings
   (fastembed / candle). This is the 2026 accuracy lever for the natural-language queries the README admits are the
   weak spot — gated behind the `embed` Cargo feature, bit-identical output when off.
2. **New direction: graph-RAG over the tree-sitter index.** Promote the symbol index to a **code-context graph**
   using the calls/refs/imports edges CallSieve already extracts. This answers "where do we handle the case where the
   user cancels mid-flow" *deterministically* (no embeddings) — RepoGraph-style results, but local + zero-cloud. The
   "Reliable Graph-RAG" benchmark says the deterministic AST-graph is the right build for exactly CallSieve's constraints.
3. **AST-aware chunking for the embedding layer.** When embeddings land, chunk by **AST node** (function / class /
   block), not by line — the Wholembed-v3 approach — so the semantic layer respects code structure and matches
   symbol boundaries the deterministic ranker already uses.

## What stays CallSieve's moat

Deterministic · local · zero-cloud · auditable · first-correct-file-proven. The 2026 research **validates all four**,
and the contamination / benchmark-saturation findings confirm `first_correct_file_rate` is the *right* headline metric
to lead with. The upgrades above are additive: every one preserves the "no embeddings, no surprises" determinism
guarantee and the zero-model-token retrieval promise.

---

*Sources (June 2026): RepoGraph · CodexGraph · GraphCoder · Codebase-Memory (arXiv 2603.27277) · Reliable Graph-RAG
for Codebases · Wholembed-v3 · ColBERT / ColPali / SPLADE late-interaction (ColBERT-Att 2603.25248) · "One Tool Is
Enough" (2512.20957) · Agentic Context Engineering (ACE) · SoK: Agentic RAG (2603.07379) · LLM Issue-Resolution
survey (2601.11655). Compiled from a 9-round SOTA research pass; full notes in the companion GLM-5.2-Demolition repo.*
