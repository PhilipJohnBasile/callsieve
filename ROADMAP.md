# CallSieve Roadmap - "Beat the built-ins"

This roadmap captures the six bets we are making to move CallSieve from
"infrastructure with a proof appendix" to "the thing that makes your agent
stop being dumb about your codebase." Client breadth is a constraint, not a
variable: every bet here assumes we keep supporting Codex, Claude Code,
Copilot, OpenCode, Antigravity, Cursor, VS Code, Windsurf, Continue, Zed,
Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo, Roo, and generic
MCP.

The plan is structured as six parallel workstreams. Each workstream is owned
by one agent on a small internal team. Workstreams are independently
shippable - none blocks another - but they reinforce each other (the
positioning shift gives the benchmark something to claim; the benchmark gives
the commercial motion something to sell; etc.).

---

## Team

| Agent | Workstream | Primary surfaces |
| --- | --- | --- |
| Positioning Agent | 1. Reframe the pitch | `README.md`, `PRODUCT_BRIEF.md`, `docs/*`, demo flows |
| Retrieval Agent | 2. Hybrid retrieval | `src/query/ranker.rs`, new `src/query/embed.rs` |
| Indexer Agent | 3. Behavioral signals in the index | `src/indexer/*`, new `src/indexer/git.rs`, `src/indexer/ownership.rs`, `src/indexer/runtime.rs` |
| Editor Agent | 4. Editor-first sidebar | `editors/vscode/`, `editors/cursor/`, MCP bridge |
| Benchmark Agent | 5. Public third-party benchmark | `benchmarks/public/`, CI harness |
| Commercial Agent | 6. Money story | `commercial/`, dashboard repo, audit-log integration |

Workstreams are listed in roughly the order they unlock value; they are not
sequential.

---

## 1. Reframe the pitch from "save tokens" to "first try is the right try"

### Goal
Lead with what developers actually feel - wrong-file rate, time-to-correct-
edit, turns-to-resolution - and demote `context_payload_reduction` to a
secondary CFO-facing number.

### Why
Tokens are abstract. Devs install tools that visibly fix a pain. The current
top-of-fold is about cost; that's the buyer's story, not the user's. We keep
the cost story for enterprise, but we lead with speed and accuracy.

### Metric definitions (precise)
- `first_correct_file_rate` - fraction of tasks where the top-K read-first
  packet contains at least one file that the ground-truth patch modifies.
  K defaults to 5 and is reported alongside the metric.
- `turns_to_first_edit` - number of agent tool calls between session start
  and the first `Edit`/`Write` against a file that is also in the final
  merged patch. Lower is better. Counts only files in the ground-truth
  patch so that exploration into wrong files is penalized.
- `wrong_files_read` - count of distinct files the agent fully read
  (`Read`, full-file `cat`, full-file grep with context) that the
  ground-truth patch did not modify.

Ground truth comes from public-benchmark fixtures (workstream 5) or, for
observed sessions, from the merged PR diff after the fact.

### Workstreams
1. **Instrument the metrics.** Extend the observed-session trace schema in
   `src/cli.rs` and `src/output/json.rs` so `session-event` records the
   tool kind (`read`, `edit`, `write`, `grep`, `glob`, `context`) and the
   absolute path acted on. Add a new `session-finish` post-processor that,
   given a ground-truth patch file list, computes all three metrics and
   writes them to the summary JSON.
2. **Top-of-README rewrite.** New hero section in `README.md` leads with a
   30-second recording of Claude Code going from "reads 11 files" to
   "reads 3" on a real task. Move `context_payload_reduction` discussion
   below the fold under "How we measure cost."
3. **New 30-second demo script.** `callsieve demo` defaults to printing
   `first_correct_file_rate` and `turns_to_first_edit` against a small
   built-in fixture set. `context_payload_reduction` stays available under
   `--verbose`.
4. **Doc consolidation.** Rewrite top-of-page hooks for `AGENT_CLI.md`,
   `BENCHMARKS.md`, `DOGFOOD.md`, `PILOTS.md`, `OBSERVED_SESSIONS.md`,
   `ENTERPRISE_PROOF.md` to lead with the new metrics. The audit/proof
   stack stays; its framing becomes "evidence for the speed/accuracy
   claim."

### Smallest shippable slice (day one)
Add the three metric fields to the existing `session-finish` summary JSON
with a stub ground-truth-patch input (`--ground-truth-files <path>...`).
Compute them from the existing trace event list. No README change yet -
just prove the numbers are computable end-to-end on one repo.

### Success metric
- README hero metric is `first_correct_file_rate` and `turns_to_first_edit`.
- `callsieve demo` prints those two numbers by default on every supported
  repo.
- One animated demo recording exists and is linked from the README.

### Owner
Positioning Agent.

---

## 2. Hybrid retrieval - deterministic floor, optional local embeddings

### Goal
Keep the deterministic ranker as the default and the only thing that runs
when offline. Add an opt-in local embedding layer that augments (never
replaces) the ranker for harder semantic queries, with zero network
dependency and zero loss of the "no cloud" guarantee.

### Why
Deterministic ranking handles "where is X defined" well and "where do we
handle the case where the user cancels mid-flow" poorly. Queries of the
second shape are exactly where Cursor/Continue/Cody win today.

### Workstreams
1. **Embedding interface.** Add `src/query/embed.rs` with a trait
   `LocalEmbedder { fn embed(&self, texts: &[&str]) -> Vec<Vec<f32>>; fn id(&self) -> EmbedderId; }`
   and adapters for at least two backends:
   - `fastembed` (BGE-small-en, CPU, ~33M params, ~100MB on disk)
   - `candle` (BGE-small Rust-native, no Python dependency)

   Embeddings are computed at index time per symbol and per file-summary,
   stored in `.callsieve/embeds.bin` as `(EmbedderId, dim, f16 vectors)`
   alongside the JSON index, and rebuilt by `watch`/`daemon`. Cache key is
   `{model_id, model_revision, index_schema_version}`; mismatch triggers
   automatic rebuild rather than serving stale vectors.
2. **Hybrid ranker.** Modify `src/query/ranker.rs` to combine the existing
   deterministic score `d` and the cosine-similarity semantic score `s`
   using an explicit gate:
   - If query parses as an identifier (regex `^[A-Za-z_][A-Za-z0-9_]*$`,
     or <= 3 whitespace-separated tokens of which >= 1 is `CamelCase` /
     `snake_case`): final = `0.85 * d + 0.15 * s`.
   - Otherwise: final = `0.40 * d + 0.60 * s`.

   Weights live in `.callsieve/config.toml` so we can tune without a
   release. The blend choice and weights are recorded in the packet's
   `why` array so audits can trace why a file was selected.
3. **Off by default, one-line on.** `callsieve index <path> --embed
   [--embedder fastembed|candle]` builds the embedding cache.
   `callsieve agent-context` automatically uses embeddings when the cache
   exists and the embedder ID matches. No embeddings, no surprises.
4. **Determinism preservation.** When embeddings are off, every existing
   output, score, and trace is bit-identical to today's `main`. Hybrid
   mode is gated by a `retrieval_mode: "deterministic" | "hybrid"` field
   in `.callsieve/index.json` and echoed in every packet so audits can
   prove which mode produced a result.
5. **Doc the bet.** New section in `docs/AGENT_CLI.md` and `README.md`:
   "Why hybrid, not pure embeddings."

### Smallest shippable slice (day one)
Land the `LocalEmbedder` trait, one `fastembed` adapter, and the
`.callsieve/embeds.bin` writer behind a Cargo feature `embed`. No ranker
changes yet; just prove we can build and load embeddings deterministically
across runs.

### Success metric
- Hybrid mode lifts `first_correct_file_rate` by >=10 percentage points on
  the public benchmark (workstream 5) for natural-language tasks, while
  leaving symbol-shaped tasks unchanged (+/-1 pp).
- `cargo test` passes with `--no-default-features` (no embed) and
  `--features embed`.
- All existing client integrations work unchanged in both modes.

### Owner
Retrieval Agent.

---

## 3. Index more than symbols - behavioral signals

### Goal
Move from "static structure index" to "structure + behavioral context." Make
the read-first packet include who knows the code, what changes often, who
owns it, and what's failing in production - when those signals are available
locally.

### Why
Symbols + imports + tests is the easy half of "what's relevant." The harder
half - change frequency, ownership, recent breakages - is where built-in
agent context layers can't easily go.

### Workstreams
1. **Git signal - `src/indexer/git.rs`.**
   - Per-file: last-modified timestamp, commits in last 30/90 days,
     distinct-author count, churn (lines changed). Computed by one pass
     of `git log --since=90.days --numstat --pretty=format:'%H%x09%an%x09%at'`
     parsed in-process. No per-file fork.
   - Per-symbol: last-modified author and timestamp via cached
     `git blame -L <start>,<end> --line-porcelain` keyed by
     `(commit_sha, path, start, end)`. Cache lives in
     `.callsieve/git_cache.bin` and is invalidated when `HEAD` moves or
     the file's symbol ranges change.
   - Daemon updates incrementally: on file change, refresh only that
     file's git rows; on `HEAD` change, refresh the per-file aggregate
     pass.
   - Folded into the ranker as a `recency / hotspot` boost with a small
     weight (default 0.10) and an explicit `why: ["hot file"]` /
     `why: ["recently changed by <author>"]` entry.
2. **Ownership - `src/indexer/ownership.rs`.**
   - Parse `CODEOWNERS`, `.github/CODEOWNERS`, `docs/CODEOWNERS`,
     `.gitlab/CODEOWNERS` using the canonical GitHub matching semantics
     (last matching pattern wins, glob support).
   - Surface owner and team in `read_first[].ownership` so the agent can
     suggest reviewers and PR-routing tools can consume it.
3. **Runtime / error context - `src/indexer/runtime.rs`.**
   - Optional ingestion of a local export from Sentry/Datadog or a stack
     trace pasted into a file. New flag:
     `callsieve agent-context <path> "<task>" --error <file>`.
   - Stack-trace lines are matched to indexed files by suffix and to
     symbols by line range. Matched files get an explicit boost and a
     `why: ["appears in provided stack trace"]` entry.
4. **Schema bump and migration.** `.callsieve/index.json` `schema_version`
   bumps from N to N+1.
   - Forward compat: new clients reading an N-schema index treat
     behavioral fields as absent and continue to work.
   - Backward compat: old clients reading an N+1 index ignore the new
     fields silently (top-level shape is additive).
   - `status` reports which behavioral signals are populated for the
     current index.
5. **MCP surface.** `callsieve_context` returns the new fields; the JSON
   schema documented in `docs/MCP.md` is updated. No breaking change for
   the 19 supported clients - every new field is optional.

### Smallest shippable slice (day one)
Ship `src/indexer/ownership.rs` only: parse `CODEOWNERS` at index time
and surface an `ownership` field on every `read_first` entry. Smallest
risk, smallest perf impact, immediate value. Git signal and runtime
context land after.

### Success metric
- On the public benchmark, tasks with a stack trace + `--error` see >=20
  percentage points improvement in `first_correct_file_rate`.
- `ownership` field present for >=95% of files in repos that have a
  `CODEOWNERS` file.
- Git signal computation adds <10% to a cold index time on a 50k-file
  repo and <2% to incremental refresh per file.

### Owner
Indexer Agent.

---

## 4. Editor-first sidebar - meet developers before the agent session

### Goal
Ship a VS Code extension (and Cursor variant that reuses 90% of the code)
that shows the read-first set live as the developer types or selects a task,
with one-click "send to Claude Code / Cursor / Copilot."

### Why
Agent sessions are bursty; editors are constant. Today CallSieve is invisible
until a developer is already in an agent session and remembers to run
`callsieve agent-context`. The editor sidebar makes the value visible
continuously, widens the funnel, and is the natural place to surface the new
headline metrics from workstream 1.

### Workstreams
1. **Extension scaffold - `editors/vscode/`.** TypeScript extension. The
   extension spawns `callsieve mcp` as a child process and speaks
   JSON-RPC over its stdio pipes - no new transport, no new ports, no
   sockets to manage. If `callsieve mcp` fails to start, the extension
   falls back to shelling out to `callsieve agent-context --format json`.
2. **Sidebar UI.** Input box for the task; live-updated list of
   read-first files with score, symbols, blast radius, ownership (from
   workstream 3), and "why selected" hints. Renders incrementally as the
   user types (debounce 250 ms).
3. **One-click send.** Buttons:
   - "Send to Claude Code" - invokes the Claude Code VS Code command.
   - "Send to Cursor composer" - uses Cursor's `composer.openWith` API.
   - "Copy as Copilot prompt" - clipboard with the packet rendered as a
     prompt prefix.
   - "Copy as Markdown" - universal fallback for any other agent.
4. **Cursor variant.** Same extension source, Cursor-specific publishing
   manifest and command-palette registration. We keep parity, not a
   fork.
5. **Health/freshness indicators.** Status-bar item shows index age,
   daemon status, and one-click "refresh now" -> `callsieve daemon --once`.
6. **Telemetry - local only by default.** Sidebar computes
   `first_correct_file_rate` from what the developer actually opens
   after a packet is returned. Stored locally in
   `.callsieve/local-metrics.jsonl`. A clearly labeled opt-in toggle
   "Send to team dashboard" hooks into workstream 6; off by default,
   surfaced once with a one-time notice. No surprise uploads.

### Smallest shippable slice (day one)
Minimal VS Code extension that spawns `callsieve mcp`, shows a sidebar
with the read-first list for a typed task, and a single "Copy as
Markdown" button. No Cursor variant, no telemetry, no fancy send
buttons. Prove the loop end-to-end on one editor.

### Success metric
- Extension shipped to the VS Code Marketplace and Cursor's extension
  channel.
- Median time from "type task" to "useful read-first list" under 500 ms
  on a warm index.
- A new "install the sidebar" path appears in `docs/INSTALL.md`
  alongside the existing CLI install.

### Owner
Editor Agent.

---

## 5. Public third-party benchmark - the credibility move

### Goal
Publish one number, on real public repos, that a skeptical engineer can
reproduce on their own machine. SWE-bench-style: known issues, known repos,
agent runs with-and-without CallSieve, metrics that match the new headline
in workstream 1.

### Why
The internal proof stack is impressive and exhaustive but it reads as "we
are worried no one will believe us." One credible public number beats a
hundred internal `proof-report` JSONs. This is the move that lets the
positioning shift land.

### Two benchmark modes (honest about reproducibility)
We split this into two modes because end-to-end agent runs are expensive
and non-deterministic; retrieval-only is cheap and bit-reproducible.

- **Mode A - Retrieval-only.** Given the ground-truth set of files
  patched in the resolved PR, measure recall@K of the read-first packet.
  No LLM is invoked. Fully deterministic, runs in CI nightly, costs ~$0.
  **This is the headline reproducibility claim.**
- **Mode B - End-to-end agent run.** Run a real Claude Code / Cursor /
  Copilot session against the bug, with pinned model, temperature 0,
  and a fixed seed where the agent supports it. Expensive (~$10-$50 per
  issue), non-deterministic at the margin, run weekly not nightly. **This
  is the marketing claim that informs the blog post.**

### Workstreams
1. **Benchmark selection.** 50 known issues across 10 well-known OSS
   repos spanning Rust, Python, TypeScript, Go, Java. Bias toward repos
   with reproducible bug fixes and clear ground-truth patched files
   (SWE-bench Lite is a good seed pool). Land selection in
   `benchmarks/public/manifest.json` with each entry pinned to a base
   commit and the resolving PR's patched-file list.
2. **Harness (Mode A).** Wrap `callsieve trace-replay` and a recall@K
   evaluator. Each issue runs three ways: baseline keyword grep, CallSieve
   deterministic, CallSieve hybrid. Output one
   `benchmarks/public/results/mode-a-<date>.json` per run.
3. **Harness (Mode B).** A separate driver under
   `benchmarks/public/mode-b/` that boots a real agent session per issue,
   captures the trace, and computes the headline metrics from workstream 1.
   Pinned model versions, pinned prompts, recorded session JSON checked in.
4. **Public metrics.** Per-issue and aggregate:
   `first_correct_file_rate@5`, `wrong_files_read`, `turns_to_first_edit`,
   wall-clock seconds, and the existing `context_payload_reduction`.
5. **Reproducibility (Mode A).** `make bench-public` clones the fixed
   repos at pinned commits, runs Mode A, and produces a results file
   bit-identical to the published one. No LLM key required.
6. **Publication.** Results live under `benchmarks/public/results/` and
   are referenced from the README hero. One short blog post walks through
   methodology, one example task, and both modes.
7. **Ongoing CI.** Nightly GitHub Action runs Mode A on `main` and posts
   a diff comment when any headline metric regresses by >2 pp. Mode B
   runs weekly on a separate workflow with explicit cost gating.

### Smallest shippable slice (day one)
Land `benchmarks/public/manifest.json` with 5 issues from a single repo
plus the Mode A recall@K evaluator. One JSON output file, no CI yet,
no Mode B. Prove that we can produce a reproducible number end-to-end
before scaling to 50 issues.

### Success metric
- One public benchmark page with a single dominant chart that a reader
  can grok in 10 seconds.
- A third party (not us) reproduces the Mode A headline number within
  1% (bit-exact modulo float order); reproduces Mode B within 5% on an
  identical-class agent run.
- CI guards against regressions on every PR that touches `src/query/`
  or `src/indexer/`.

### Owner
Benchmark Agent.

---

## 6. Commercial story - three concrete bets, not "paid pilots"

### Goal
Replace the vague "paid pilots" motion with three concrete commercial
products, each with a clear buyer, install path, and price.

### Why
"Custom pilot" is slow and bespoke. We need a price page. Without a clear
commercial wedge the open-source core has no funded engine behind it.

### Workstreams
1. **Free local CLI - unchanged.** Stays MIT, stays the front door.
   Every commercial product layers on top, never replaces.
2. **Paid product A - team dashboard (`commercial/dashboard/`).**
   - V1 delivery model: **self-hosted single-binary**, runs against
     local SQLite, no cloud dependency. Hosted cloud is V2 once at least
     three self-hosted customers exist.
   - Aggregates `proof-report` JSON from many developers across many
     repos.
   - Surfaces per-team `first_correct_file_rate`,
     `turns_to_first_edit`, retrieval-quality drift alerts, and token
     spend deltas.
   - Buyer: engineering leaders who want to know whether their agent
     investment is working.
   - Placeholder price: **$20/developer/month**, billed annually,
     unlimited repos, support included.
3. **Paid product B - audit log for security / compliance.**
   - Every agent action, every file read, every file CallSieve *would*
     have surfaced, as a JSONL stream Splunk/Datadog/Sumo can ingest
     directly. SIEM connectors ship as separate small binaries.
   - Buyer: security teams at SOC2/FedRAMP/regulated-industry orgs that
     need to answer "what did the AI agent touch?"
   - Placeholder price: **$500/repo/year**, free tier for repos with
     <=5 active developers in the trailing 30 days (auto-detected from
     `git log`).
4. **Paid product C - monorepo retrieval tuning (services).**
   - Hands-on engagement for repos >100k files where the default ranker
     needs tuning, custom signal ingestion, and benchmark validation.
   - Buyer: large engineering orgs with a single huge repo.
   - Placeholder price: **$25k fixed-fee engagement**, productized over
     time as repeatable patterns emerge.
5. **Pricing page.** A real `commercial/PRICING.md` (and eventually a
   website page) with the three products, buyers, and numbers above.
   No "contact sales for pricing" on the entry tiers - only the
   monorepo engagement requires a conversation.
6. **Path from free to paid.** `callsieve proof-report` outputs a
   one-line pointer: "Aggregate across your team with the CallSieve
   dashboard -> <link>." No dark patterns; just a real upgrade path. The
   editor sidebar's "Send to team dashboard" toggle (workstream 4) is
   the other entry point.

### Smallest shippable slice (day one)
Write `commercial/PRICING.md` with the three products and placeholder
numbers above. No dashboard binary yet - just publish the page so we
can start having real pricing conversations with prospects this week.

### Success metric
- Public pricing exists for at least two of the three products.
- At least one signed paid customer on product A or B (real contract,
  not a pilot LOI).
- The open-source repo is unchanged in scope; no code is moved behind a
  paywall.

### Owner
Commercial Agent.

---

## What we are explicitly not doing

- **Cutting client support.** Codex, Claude Code, Copilot, OpenCode,
  Antigravity, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI
  Assistant, Amp, Goose, Warp, Cline, Zoo, Roo, and generic MCP all stay
  supported. Every workstream above must work across all of them.
- **Moving any feature behind a paywall.** The OSS core grows. Commercial
  products are net-new layers above it.
- **Adopting embeddings as the default.** Hybrid retrieval is opt-in and
  the deterministic floor remains the audit-friendly default.
- **Cloud retrieval.** Local-first guarantees in the README stay verbatim.

---

## Cross-cutting acceptance criteria

Every workstream is "done" only when:

1. All 19 supported clients still pass `callsieve doctor --strict`.
2. `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
   `cargo fmt --check` all pass on `main`, with and without optional
   Cargo features (`embed`, etc.).
3. The public benchmark Mode A (workstream 5) shows no regression on
   `first_correct_file_rate@5` versus the prior `main`. Mode B may
   move within +/-2 pp run-to-run noise.
4. The README's new headline metrics are still front and center.
5. Any change to `.callsieve/index.json` is additive (forward- and
   backward-compatible) or bumps `schema_version`. Stale caches are
   rebuilt automatically rather than served.

---

## How the team works

Workstreams run in parallel. Each agent owns their surface end-to-end:
code, tests, docs, and the relevant section of `README.md`. Cross-workstream
changes (e.g., the indexer adding fields the editor sidebar will render)
land behind a feature flag in the index schema so neither side blocks the
other.

When a workstream collides with another (e.g., the ranker change in
workstream 2 and the new behavioral signals in workstream 3 both touch
`src/query/ranker.rs`), the owning agents coordinate via a short design
note in `docs/design/` rather than a long meeting.
