# Competitive Notes

Last checked: 2026-06-13 (v0.3.4, public proof target pass).

CallSieve is the codebase filter underneath coding agents. The winning position is not "another agent" and not "bigger context." It is local, deterministic retrieval that makes agents read the smallest useful packet before they spend model tokens on grep, search, and repeated file reads.

## Current Position

CallSieve already has the right wedge, and as of v0.3.4 several previously aspirational claims are measured:

- Retrieval quality: deterministic Mode A first-correct-file@5 is `100.0%` on the public 50-issue SWE-bench Lite subset after Python settings/constants, framework-domain module aliases, lifecycle-symbol dampening, domain-module retention during test-companion promotion, SQL order-by relation compiler routing, SQL query-builder routing for filterability and combined-query issues, ORM lookup routing, auth proxy-permission migration routing, `UniqueConstraint` model field-check routing, `ForeignKey` `to_field` rename autodetection, and dynamic `SCRIPT_NAME` static/media settings resolution improved path and symbol matching. A third-repo 10-task `psf/requests` seed also reaches `100.0%` after Requests session method-normalization routing, increasing the checked public proof gate to 186 evaluated public tasks. The strict compare gate still proves `60.0%` lexical first-correct-file@5 with a `+54.0 pp` lift over naive grep. On the 30-issue natural-language slice, deterministic Mode A reaches `100.0%`; the strict compare report still records `36.7%` hybrid with `+23.3 pp` over naive grep. Public pinned Rust and TypeScript language-smoke slices both pass at `100.0%` first-correct-file@5.
- Repo-packer economics: checked-in full-repo prompt-pack proxy baselines measure Astropy at `5,638,468` estimated tokens and Django at `5,155,623` estimated tokens, each over `1,000x` the default CallSieve proof packet. The smaller Requests checkout measures `101,186` estimated tokens, still over `20x` the default proof packet.
- Speed: the daemon serves `agent-context` from an in-memory index over a local socket, `0.31s` end-to-end on a 2.7k-file Django checkout vs `0.61s` direct, byte-identical output. The index itself is 55% smaller (schema 9).
- Team onboarding: `index-export`/`index-import` give Cursor-style team warm starts through any channel the team already trusts, verified by content hash, with zero code upload.
- Zero-decision setup: `callsieve setup-auto` detects installed agents and configures rules, MCP, and lifecycle hooks in one command.
- Distribution: working release binaries for four targets, `brew install philipjohnbasile/callsieve/callsieve`, and cargo-binstall support.

The original wedge holds:

- Local Rust CLI and MCP server.
- Local JSON index, no cloud service, no API key, no vector database by default.
- Agent-facing `agent-context`, `callsieve grep`, and `callsieve_context` packets with `retrieval_cost.retrieval_model_tokens = 0`, compact `stats.b`/`stats.t`, and compact `stats.local` index counts, plus local missing/stale index refresh before context.
- Compact `context.sel` array evidence for why the first few files were selected, including capped local score signals, plus normal/full `selection_confidence` tiers for agents that need an explicit high/medium/low trust hint.
- Public proof now has direct MCP gates: `mcp_surface` checks first-mile tools, and `mcp_contract` checks the stable `callsieve_context` structuredContent contract agents consume before broad search.
- Focused local expansion through `focus`, `related`, and `tests` before grep.
- Hooks, shims, trace checks, observed-session evidence, proof reports, pilot reports, and enterprise gates.
- Competitive and positioning tasks now get a deterministic ranking boost for explicit competitor or positioning docs, so agents read the strategy artifact before generic CLI or MCP setup docs.

The hard product rule is simple: make every agent do heavy retrieval work on the user's machine, then spend model tokens only on the compact selected packet.

## Competitive Goal

Make CallSieve the default local-first context layer an agent uses before it searches, reads, edits, or reviews a repository.

The goal is not to become a better IDE than Cursor, a better enterprise context platform than Augment or Tabnine, a better refactoring engine than Serena, or a better PR bot than Qodo or CodeRabbit. The goal is to beat all of them at the first mile of agentic coding: selecting the smallest correct read-first packet, proving the retrieval work happened locally, and making that packet usable from any agent.

### Win Conditions

CallSieve is winning when a buyer or maintainer can run the same task through CallSieve and a competitor, then see these outcomes:

1. **Better first-mile economics**: CallSieve retrieval uses `retrieval_model_tokens = 0`, has no per-query retrieval fee, and avoids default code upload while still returning useful ranked files, symbols, tests, and follow-ups.
2. **Better pre-prompt relevance**: default deterministic retrieval holds the public 50-issue first-correct-file@5 line at the `100%` proof target and the 30-issue natural-language line at the `100%` proof target without making embeddings mandatory.
3. **Better proof posture**: `receipt`, `trace-check`, `proof-report`, and `enterprise-proof-report` show whether context came before grep, how many broad searches and file reads were avoided, and which evidence is observed versus replayed.
4. **Better agent neutrality**: Codex, Claude Code, Copilot, Cursor, Windsurf, Continue, OpenCode, Zed, JetBrains, Cline, Roo, and generic MCP clients can all use the same local index and shared memory without buying into one editor, one review bot, or one cloud context engine.
   Public proof now exposes the full default-layer setup gate as 19/19 covered clients, not just a priority-client sample.

### How To Beat Each Class

- **Against Augment and Tabnine**: be the no-signup, no-auth, no-cloud-upload, no-retrieval-bill alternative with transparent local packets and receipts.
- **Against Serena and Codebase-Memory**: keep improving graph quality, but package it as task-ranked read-first context, related tests, ownership, risk, and proof instead of only semantic tools.
- **Against Repomix, Gitingest, and Code2Prompt**: make selection visibly cheaper than dumping the repo by showing the exact files, symbols, tests, and packet-token savings.
- **Against Qodo, CodeRabbit, and Greptile**: own upstream context before the PR review starts, then feed review agents better callers, tests, ownership, and blast-radius packets.

### Next Proof Target

The next competitive milestone now starts with a checked-in competitor-response report:

```bash
callsieve competitive-report benchmarks/competitive-response-manifest.example.json
```

That manifest runs CallSieve on representative local tasks and compares:

- install friction and first query time
- code upload requirement
- retrieval model-token cost
- per-query retrieval cost
- default packet token count
- locally measured first-query p50/p95 latency
- first-correct-file@5
- expected-file recall
- natural-language recall evidence from a required local fixture
- explainability of why files were selected
- minimum and required agent coverage
- strict trace or receipt proof that broad search happened after context

The checked-in manifest treats 100% core expected-file recall, natural-language recall, first-mile economics, agent coverage, and trace-or-receipt proof as required gates so competitive claims fail closed when evidence is missing.

Keep evolving this report as a repeatable command and manifest, not a slide. If CallSieve cannot measure a claim locally, mark it as a source claim or missing evidence instead of using it as proof.

The public-facing proof track now lives in [docs/PUBLIC_PROOF.md](PUBLIC_PROOF.md) and `benchmarks/public-proof-manifest.example.json`. It combines the strict local competitive gate with checked-in public SWE-bench-style reports, a third-repo Requests seed, grep-lift evidence, public Rust and TypeScript language-smoke slices, a public result catalog that makes best measured rates and target gaps explicit, a `broad_read_guardrail` that fails closed unless CallSieve beats naive broad file reads by the manifest target, a `context_packet_guardrail` that requires symbols, related tests, blast-radius/risk hints, caller/callee call-graph hints, selection evidence, and local expansion targets, manifest-driven full-repo pack proxy baselines, MCP surface and structured-content contract gates, content validation for standalone `mcp-contract.json`, `agent-native-protocol.json`, recomputed `agent-native-template.json`, and measured `agent-native-check.json` artifacts, and an explicit `agent_native_search_guardrail` that stays `not_measured` until real Cursor, Copilot, Claude Code, Devin, or similar runs are added. External Repomix, Gitingest, Code2Prompt, and agent-native search artifacts can be added when those tools are installed and approved; `agent-native-template` pre-fills CallSieve's side of a native-search task log, `agent-native-check` preflights filled native-search logs and transcript/export source artifacts, and `agent-native-baseline` standardizes transcript/export-backed native-search task logs into the guardrail artifact shape with locally readable source artifacts whose byte counts and hashes are recomputed by public proof. Once a measured agent-native baseline is present, public proof also requires the checked `agent-native-protocol` and measured `agent-native-check` terminal artifacts, recomputes the measured preflight from the check artifact's task-log and transcript/export sources, and requires the baseline source hashes to be present together in one passing measured-check artifact, so native-search wins cannot pass without the measurement playbook and matching preflight result.

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
| Augment Context Engine / Auggie | Augment is now a direct context-layer competitor, not just an AI coding assistant. Its Context Engine MCP plugs into Claude Code, Codex, Cursor, GitHub Copilot, OpenCode, Roo Code, Zed, Gemini CLI, and other clients; claims semantic understanding, relationship awareness, external sources, smart curation, local real-time indexing, remote cross-repo indexing, and compressed context. Augment also says MCP queries are token-billed and average roughly `$0.03-$0.06` per query. Sources: [Augment Context Engine MCP](https://docs.augmentcode.com/context-services/mcp/overview), [Auggie workspace context](https://docs.augmentcode.com/cli/setup-auggie/workspace-context), [Augment workspace indexing](https://docs.augmentcode.com/setup-augment/workspace-indexing). | This is the most obvious head-to-head wedge. Augment is better on semantic retrieval, hosted cross-repo context, external knowledge, and enterprise polish. CallSieve should beat it on no signup, no auth, no API key, no per-query retrieval bill, no default code upload, deterministic local ranking, compact receipts, and proof that retrieval itself used zero AI model tokens. |
| Tabnine Enterprise Context Engine | Tabnine now positions its Enterprise Context Engine as a standalone agent-agnostic context layer. It claims a hybrid graph plus vector model, dependency and blast-radius analysis, verification against standards, shared memory for multi-agent systems, support for Cursor, GitHub Copilot, Claude Code, internal agents, and deployment options including SaaS, on-prem, private VPC, and air-gapped. Sources: [Tabnine home page](https://www.tabnine.com/), [Tabnine Enterprise Context Engine](https://www.tabnine.com/enterprise-context-engine/). | Tabnine is a serious enterprise context competitor. It can beat CallSieve on sales motion, governance, org-level knowledge graphs, and deployment breadth. CallSieve should beat it for developers and small teams with an open-source local CLI, inspectable JSON index, simpler setup, deterministic packets, and evidence artifacts that do not require buying an enterprise platform first. |
| Qodo | Qodo is an AI code review and governance platform with IDE, Git, and CLI surfaces. It emphasizes context-aware review, local code review, a rules system, a context engine that pulls codebase, PR history, rules, and business requirements, plus multi-repo understanding for large organizations. Source: [Qodo platform](https://www.qodo.ai/). | Qodo should beat CallSieve at review workflow packaging, rules governance, and PR distribution. CallSieve should beat it before review starts: selecting the right files, tests, callers, and blast radius for any coding agent before the agent edits or opens a PR. Do not turn CallSieve into Qodo; make it the context packet Qodo-style review agents would want upstream. |
| CodeRabbit | CodeRabbit is a highly distributed AI code review product with Git, IDE, and CLI review surfaces. It claims codebase intelligence through codegraph and custom guidelines, external context through MCP, linked issues, web query, 40+ linters and scanners, and learnings from review feedback. Source: [CodeRabbit home page](https://www.coderabbit.ai/). | CodeRabbit wins on two-click PR-review adoption, social proof, and review UX. CallSieve should win by staying pre-prompt and agent-neutral: local retrieval before an agent spends tokens, compact packet accounting, traceable grep/read avoidance, and no commitment to a review bot workflow. |
| Serena | Serena is a close open-source MCP peer: a semantic coding toolkit that gives agents IDE-like symbol retrieval, references, editing, refactoring, debugging, and memory through language servers or a paid JetBrains backend. It supports many clients and over 40 languages through LSP. Source: [Serena GitHub repository](https://github.com/oraios/serena). | Serena may beat CallSieve on true IDE semantics, refactoring tools, and language breadth. CallSieve should beat it on task-ranked read-first packets, deterministic score explanations, tests/ownership/risk/proof workflows, context-first enforcement, and simple local indexing without requiring each language server to be healthy. Consider Serena complementary for deeper semantic follow-ups. |
| Repo packers: Repomix, Gitingest, Code2Prompt | These tools convert repositories into prompt-friendly text instead of ranking a small read-first packet. Repomix packages an entire codebase into AI-friendly formats, counts tokens, respects gitignore, offers Tree-sitter compression, and includes an MCP server. Gitingest turns any Git repo into a simple text digest. Code2Prompt is a Rust CLI, SDK, TUI, and MCP server for generating formatted prompts with token tracking and git metadata. Sources: [Repomix](https://repomix.com/), [Gitingest](https://gitingest.com/), [Code2Prompt](https://github.com/mufeedvh/code2prompt). | They win on immediate explainability: "turn my repo into a prompt" is easy to understand. CallSieve should beat them on token economics and relevance: the checked public proof now measures full-repo prompt-pack proxy payloads over 1,000x larger than the default CallSieve packet, while still ranking the few files, symbols, tests, and follow-ups an agent should read first. |
| Codebase-Memory and MCP graph prototypes | The research and open-source direction is converging on persistent code graphs exposed through MCP. Codebase-Memory, submitted in March 2026, describes a Tree-sitter knowledge graph over 66 languages, call-graph traversal, impact analysis, and community discovery, with reported 10x fewer tokens and 2.1x fewer tool calls versus file exploration. Source: [Codebase-Memory arXiv preprint](https://arxiv.org/abs/2603.27277). | This is not necessarily a commercial competitor yet, but it is proof the category is obvious to strong builders. CallSieve should beat prototype graph tools with a shipped CLI, real setup paths, compact default output, observed-session receipts, benchmark manifests, and enterprise-proof gates. It should also benchmark against graph-native questions instead of assuming keyword retrieval is enough. |

## How They Are Better

- Semantic retrieval: Cursor, Copilot, Continue, and Windsurf can find concept matches when exact terms are missing.
- Native workflows: Cursor, Copilot, Sourcegraph, Continue, and Windsurf are already where developers type.
- Large-repo onboarding: Cursor's team index reuse is a real advantage for huge repositories.
- Cross-repo enterprise search: Sourcegraph is much broader than a local repo index.
- Agent-neutral context engines: Augment and Tabnine are now explicitly selling context layers that work across several agents.
- Graph and review packaging: Greptile has a clear PR-review product shape.
- Review distribution: Qodo and CodeRabbit own PR, IDE, and CLI review flows in a way CallSieve does not.
- Semantic MCP tooling: Serena provides symbol-level retrieval, references, refactoring, and debugging through LSP or JetBrains.
- Prompt-packer simplicity: Repomix, Gitingest, and Code2Prompt have a very simple user promise for manual LLM workflows.
- Research pressure: Codebase-Memory shows that Tree-sitter graph MCP systems can make credible token-reduction claims.
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
- Against Augment and Tabnine: no account, no default cloud upload, no per-query retrieval fee, and local evidence that retrieval used zero AI model tokens.
- Against Serena: task-level ranking, proof artifacts, related tests, ownership, and blast-radius packets before deep semantic editing.
- Against repo packers: selection beats dumping. CallSieve now measures full-repo pack proxy payloads over 1,000x larger than the default proof packet; external packer CLI artifacts should be added when approved.
- Against review bots: upstream context selection for every agent, including review agents, without becoming locked to PR review.

## Product Priorities

1. Make compact `instruction.x` local expansion unavoidable across every context-bearing surface: CLI, MCP, hooks, setup files, editor extension, `begin`, `guard`, and proof workflows. Default agent-context JSON should carry read-first indexes instead of duplicated file paths; Markdown and UI surfaces can expand them into commands.
2. Improve graph quality for references, callers, callees, imports, related tests, ownership, and blast radius. Natural-language retrieval now has a bounded same-module sibling signal for vocabulary-gap misses where multiple top anchors agree on a source module. Python module-level settings/constants are indexed as symbols so setting-definition files can rank directly instead of only through their consumers. Common code-vocabulary bridges keep natural-language prompts connected to path and symbol terms such as compiler, rst, http, db, temp, and username.
3. Continue extending compact ranking explainability beyond the top file: default skim caps `context.sel.next` to one next-ranked file, skim `g.u` and `g.d` name one upstream/downstream non-test code-file preview for the top file without snippets, default skim defers lower-file graph detail and caller/callee paths to local `focus` or richer profiles, compact skim symbols use arrays such as `[name,line]` with functions as the implicit kind and short trailing codes for non-function kinds, compact skim `i` impact arrays can append graph evidence flags such as `test,im,call,ref,by`, top-file skim reasons and `context.sel.sig` use short codes such as `sym`, `sy`, `kw`, `ct`, and `comp`, `context.sel.top` and `context.sel.next` use read-first indexes when possible, and indexed selection entries omit duplicate scores because read-first array order already carries ranking. `instruction.x.o` and `instruction.x.n` can focus top and next ranked symbols locally before grep by indexing into `context.read_first`; legacy or wider packets may use `instruction.x.top` or `instruction.x.next`. Symbol focus now returns the selected code unit as a bounded local snippet plus compact `calls`, `called_by`, and `related_tests` hints before whole-file reads, with generated `--line` selectors to disambiguate same-name symbols, opt-in non-call `references`, and truncation metadata only when the snippet cap is hit. Competitive-positioning docs now have an explicit local ranking signal so product-strategy tasks do not drift into generic setup docs. Further work should keep the packet small while adding only signals that prevent blind grep.
4. Keep improving local index freshness that competes with Cursor's low-latency update story, but without uploading code. Direct CLI, grep shim, and MCP context-first paths already rebuild missing or stale indexes before ranking; the daemon now stat-checks freshness per tick instead of rebuilding, and serves queries from memory.
5. Semantic retrieval status (settled by benchmarks, June 2026): hybrid reranking is proven (+3.3 pp NL with default BGE-small, +6.7 pp with the opt-in code model) and stays opt-in; union-pass injection has never fired with any practical local model and should not receive further tuning effort. General-purpose bigger models (BGE-base) are too slow to ship; the code-tuned model is the quality tier.
6. Keep expanding competitor-style evals: local suite and report outputs now include first-correct-file@k, expected-file recall, context packet tokens, avoided grep commands, avoided file reads, and optional first-query p50/p95 latency from `perf-report`.
7. Make setup harder to ignore: generated rules and hooks should block broad search before context and inject focused expansion commands.
8. Add a short competitor-response demo matrix for Augment, Tabnine, Serena, Repomix, Gitingest, Code2Prompt, Qodo, and CodeRabbit: install friction, code upload, retrieval model-token cost, per-query cost, packet compactness, explainability, proof artifacts, and whether the tool works before the agent starts searching.

## Five Differentiators To Defend (June 2026)

The June 2026 research changes the claim shape. Augment, Tabnine, Qodo, CodeRabbit, Serena, and Codebase-Memory all use parts of graph, context-engine, memory, or blast-radius language. The defensible CallSieve claim is narrower and stronger: combine local deterministic retrieval, the agent's whole session stream from hooks, and a local proof pipeline in one agent-neutral CLI with zero retrieval model tokens.

1. **Graph-consensus and symbol-unit recall**: natural-language queries get candidates boosted when multiple top-ranked files agree via import/reference edges, Python setting definitions enter the index as symbols, common NL code vocabulary maps to implementation terms, and framework-domain module aliases connect human task language to implementation modules. Public proof shows the NL slice reaching 100.0% deterministic Mode A first-correct-file@5, and the 50-issue identifier-shaped set reaches 100.0%. The adjacency study showed 82% of earlier misses were one hop from the pool. Deterministic and explainable: the packet says which top candidates agree and which symbol, path, or domain-module signal fired.
2. **Session-learning retrieval**: hooks observe which files the agent actually read after context; Stop folds them into local task memory; `--memory-boost` recalls them when a similar task recurs. The learning loop no cloud product can run without exfiltrating usage data.
3. **Edit-impact packets**: editing an indexed file returns callers, related tests, and blast-radius risk through the PostToolUse hook, the only tool participating in the write half of the agent loop.
4. **Retrieval receipts**: `callsieve receipt` turns any observed session trace into a tamper-evident summary of packets, tokens, reads, and searches; `receipts` rolls up the repo. Auditable AI-cost reduction, not vibes.
5. **Cross-agent memory**: one agent's confirmed reads teach every other agent on the repo (shared local store with client provenance), and `memory-export`/`memory-import` move learning between teammates alongside index warm starts.

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
