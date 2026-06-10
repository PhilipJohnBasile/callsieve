# Competitive Notes

Last checked: 2026-06-10 (v0.3.2).

CallSieve is the codebase filter underneath coding agents. The winning position is not "another agent" and not "bigger context." It is local, deterministic retrieval that makes agents read the smallest useful packet before they spend model tokens on grep, search, and repeated file reads.

## Current Position

CallSieve already has the right wedge, and as of v0.3.2 several previously aspirational claims are measured:

- Retrieval quality: lexical first-correct-file@5 is `60.0%` on the public 50-issue SWE-bench Lite subset (rarity-scaled filename-stem ranking, +4.0 pp over v0.3.1 with zero regressions). The opt-in code-tuned embedding model (`--embed-model code`) reaches `26.7%` vs `20.0%` lexical on the 30-issue natural-language slice.
- Speed: the daemon serves `agent-context` from an in-memory index over a local socket — `0.31s` end-to-end on a 2.7k-file Django checkout vs `0.61s` direct, byte-identical output. The index itself is 55% smaller (schema 9).
- Team onboarding: `index-export`/`index-import` give Cursor-style team warm starts through any channel the team already trusts, verified by content hash, with zero code upload.
- Zero-decision setup: `callsieve setup-auto` detects installed agents and configures rules, MCP, and lifecycle hooks in one command.
- Distribution: working release binaries for four targets, `brew install philipjohnbasile/callsieve/callsieve`, and cargo-binstall support.

The original wedge holds:

- Local Rust CLI and MCP server.
- Local JSON index, no cloud service, no API key, no vector database by default.
- Agent-facing `agent-context`, `callsieve grep`, and `callsieve_context` packets with `retrieval_cost.retrieval_model_tokens = 0`, compact `stats.b`/`stats.t`, and compact `stats.local` index counts, plus local missing/stale index refresh before context.
- Compact `context.sel` array evidence for why the first few files were selected, including capped local score signals.
- Focused local expansion through `focus`, `related`, and `tests` before grep.
- Hooks, shims, trace checks, observed-session evidence, proof reports, pilot reports, and enterprise gates.
- Competitive and positioning tasks now get a deterministic ranking boost for explicit competitor or positioning docs, so agents read the strategy artifact before generic CLI or MCP setup docs.

The hard product rule is simple: make every agent do heavy retrieval work on the user's machine, then spend model tokens only on the compact selected packet.

## Competitors

| Competitor | What they do well | Where CallSieve should beat them |
| --- | --- | --- |
| Cursor | Cursor builds a searchable codebase index for semantic search, chunks changed files syntactically, caches unchanged embeddings, uses Merkle-tree freshness, and can reuse team indexes for fast time-to-first-query. Cursor claims semantic search improved agent accuracy by 12.5% and that team reuse can cut the 99th percentile large-repo time-to-first-query from hours to seconds. Source: [Cursor secure codebase indexing](https://cursor.com/blog/secure-codebase-indexing). | They have polished IDE-native semantic retrieval and team-scale index warm starts. CallSieve should beat them on local-first control, no required cloud index, agent neutrality, explainable ranking, and proof that retrieval itself costs zero model tokens. |
| GitHub Copilot and VS Code | Copilot repository context uses semantic code search, auto-indexes repos for chat, can use semantic indexing for non-GitHub local workspaces when enterprise policy allows it, and says large repos can initially index in up to 60 seconds with quicker re-indexes. Sources: [VS Code workspace context](https://code.visualstudio.com/docs/agents/reference/workspace-context), [GitHub repository indexing](https://docs.github.com/en/enterprise-cloud@latest/copilot/concepts/context/repository-indexing). | They are deeply integrated and automatic. CallSieve should beat them by being vendor-neutral, explicit, inspectable, usable from any CLI/MCP agent, and strict about context-first enforcement before broad search. |
| Sourcegraph Cody | Cody has open-file and repository context by default and can pull context from local and remote codebases through Sourcegraph search and code intelligence. Source: [Sourcegraph Cody docs](https://sourcegraph.com/docs/cody). | Sourcegraph wins at enterprise code search and cross-repo context. CallSieve should not compete as a full code intelligence platform. It should be the tiny local pre-prompt filter that any agent can call before wasting context. |
| Devin Desktop / Windsurf Fast Context | Fast Context is a specialized retrieval subagent using SWE-grep models, parallel searches, and targeted file/section retrieval. Devin says it can retrieve relevant code up to 20x faster than traditional agentic search and uses up to 8 parallel tool calls over up to 4 turns. Sources: [Fast Context](https://docs.devin.ai/desktop/context-awareness/fast-context), [context overview](https://docs.devin.ai/desktop/context-awareness/overview). | They are attacking the same waste with a learned retrieval subagent. CallSieve should beat them on zero retrieval model tokens, local ownership, deterministic output, and agent-agnostic setup. |
| Continue | Continue has codebase awareness using embeddings and keyword search, with local default embedding generation and local index storage in legacy `@Codebase`. Sources: [Continue codebase context](https://docs.continue.dev/reference/deprecated-codebase), [Continue embed role](https://docs.continue.dev/customize/model-roles/embeddings). | Continue is strongest among local-first IDE tools. CallSieve should beat it by avoiding vector DB defaults, making retrieval explainable, enforcing context-first workflows, and producing proof artifacts rather than only better chat context. |
| Aider | Aider's repo map sends a compact, token-budgeted map of important files, symbols, and signatures to the model, using graph ranking and dynamic map sizing. Source: [Aider repository map](https://aider.chat/docs/repomap.html). | Aider is strong on token-budgeted symbol maps. CallSieve should borrow the good idea, but keep retrieval outside the agent prompt until requested, expose file/test/blast-radius packets, and work across agents instead of only inside Aider. |
| Claude Code | Claude Code intentionally uses search and file-read tools on demand instead of full codebase indexing. Source: [Claude Code FAQ](https://support.claude.com/en/articles/12386420-claude-code-faq). | This is the clearest enemy pattern: capable agent, expensive discovery loop. CallSieve should become the default local context layer Claude Code calls before its search/read loop. |
| Greptile | Greptile builds a graph of the whole repository for PR review and learns from team feedback. Source: [Greptile docs](https://www.greptile.com/docs/introduction). | Greptile wins at automated PR review packaging and graph-based impact analysis. CallSieve should not become a PR bot first. It should strengthen references, call paths, tests, ownership, and blast radius as retrieval signals for any agent. |

## How They Are Better

- Semantic retrieval: Cursor, Copilot, Continue, and Windsurf can find concept matches when exact terms are missing.
- Native workflows: Cursor, Copilot, Sourcegraph, Continue, and Windsurf are already where developers type.
- Large-repo onboarding: Cursor's team index reuse is a real advantage for huge repositories.
- Cross-repo enterprise search: Sourcegraph is much broader than a local repo index.
- Graph and review packaging: Greptile has a clear PR-review product shape.
- Token-budgeted repo map: Aider has a mature answer for always-on, compact symbol context.
- Automatic agent behavior: Copilot, Cursor, Windsurf, and Claude Code do not make users think about context plumbing.

## Where CallSieve Can Win

CallSieve should be better in the places competitors are structurally weak:

- Zero retrieval model tokens plus explicit local-work counts as a first-class contract.
- Local index by default, with no cloud service and no code upload.
- Agent-neutral CLI and MCP surface.
- Deterministic and explainable ranking.
- Compact packets by default, with explicit expansion commands before grep.
- Traceable proof that broad grep and file reads happened after context, not before.
- Evidence artifacts that separate controlled replay, observed sessions, and enterprise proof gates.

## Product Priorities

1. Make compact `instruction.x` local expansion unavoidable across every context-bearing surface: CLI, MCP, hooks, setup files, editor extension, `begin`, `guard`, and proof workflows. Default agent-context JSON should carry read-first indexes instead of duplicated file paths; Markdown and UI surfaces can expand them into commands.
2. Improve graph quality for references, callers, callees, imports, related tests, ownership, and blast radius.
3. Continue extending compact ranking explainability beyond the top file: default skim caps `context.sel.next` to one next-ranked file, skim `g.u` and `g.d` name one upstream/downstream non-test code-file preview for the top file without snippets, default skim defers lower-file graph detail and caller/callee paths to local `focus` or richer profiles, compact skim symbols use arrays such as `[name,line]` with functions as the implicit kind and short trailing codes for non-function kinds, top-file skim reasons and `context.sel.sig` use short codes such as `sym`, `sy`, `kw`, `ct`, and `comp`, `context.sel.top` and `context.sel.next` use read-first indexes when possible, and indexed selection entries omit duplicate scores because read-first array order already carries ranking. `instruction.x.o` and `instruction.x.n` can focus top and next ranked symbols locally before grep by indexing into `context.read_first`; legacy or wider packets may use `instruction.x.top` or `instruction.x.next`. Symbol focus now returns the selected code unit as a bounded local snippet plus compact `calls`, `called_by`, and `related_tests` hints before whole-file reads, with generated `--line` selectors to disambiguate same-name symbols, opt-in non-call `references`, and truncation metadata only when the snippet cap is hit. Competitive-positioning docs now have an explicit local ranking signal so product-strategy tasks do not drift into generic setup docs. Further work should keep the packet small while adding only signals that prevent blind grep.
4. Keep improving local index freshness that competes with Cursor's low-latency update story, but without uploading code. Direct CLI, grep shim, and MCP context-first paths already rebuild missing or stale indexes before ranking; the daemon now stat-checks freshness per tick instead of rebuilding, and serves queries from memory.
5. Semantic retrieval status (settled by benchmarks, June 2026): hybrid reranking is proven (+3.3 pp NL with default BGE-small, +6.7 pp with the opt-in code model) and stays opt-in; union-pass injection has never fired with any practical local model and should not receive further tuning effort. General-purpose bigger models (BGE-base) are too slow to ship; the code-tuned model is the quality tier.
6. Keep expanding competitor-style evals: local suite and report outputs now include first-correct-file@k, expected-file recall, context packet tokens, avoided grep commands, and avoided file reads; use `perf-report` for wall-clock retrieval time.
7. Make setup harder to ignore: generated rules and hooks should block broad search before context and inject focused expansion commands.

## Distribution Next Steps (need owner credentials)

- crates.io: `cargo publish` (needs a crates.io token) so `cargo install callsieve` works without `--git`.
- VS Code Marketplace: publish `editors/vscode` (needs an Azure DevOps publisher + PAT for `vsce publish`).
- MCP registry: publish the descriptor from `callsieve mcp-registry-manifest` (needs registry auth).

## Do Not Chase

- Do not build a SaaS app first.
- Do not tune union-pass semantic injection further; five experiments (cosine floors, chunk caps, BGE-base, dir-path boosts, code model) all left injection at zero. Reranking is where embeddings pay.
- Do not add auth.
- Do not add a dashboard.
- Do not make cloud indexing the default.
- Do not default to a vector database before local deterministic retrieval proves it needs help.
- Do not become another coding agent. CallSieve is the retrieval layer under all of them.
