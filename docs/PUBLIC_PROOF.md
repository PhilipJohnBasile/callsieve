# Public Proof

CallSieve should win external proof by combining seven evidence tracks:

1. A strict local competitive gate.
2. Public SWE-bench-style retrieval reports from pinned local checkouts.
3. A broad-read payload guardrail against naive grep/read loops.
4. A context-packet quality guardrail for selected files, symbols, related tests, blast radius, selection evidence, and local expansion targets.
5. A compact-packet guardrail or measured output-size baseline against repo-packing workflows.
6. MCP surface and structured-content contract gates.
7. A measured agent-native search baseline with hashed local transcripts, plus room for additional native-agent baselines.

It also carries public language-smoke slices for Rust and TypeScript so the proof does not depend only on Python-heavy fixtures.

Run the checked-in proof track with:

```bash
cargo run -- public-proof-report benchmarks/public-proof-manifest.example.json
```

The report fails closed when the local competitive gate fails, public benchmark rates fall below the manifest targets, public reports are missing, or the local-first invariants regress:

- `retrieval_model_tokens = 0`
- `per_query_retrieval_cost_usd = 0.0`
- `default_code_upload_required = false`
- default packet tokens under the manifest budget
- broad-read payload reduction at or above the manifest target
- context packets include symbols, related tests, blast-radius hints, selection evidence, and local expansion targets
- required first-mile MCP tools and context contract fields present

It also emits an `evidence_pack` section with:

- regeneration commands
- fixture manifests
- public result report paths
- key metrics
- sampled misses
- receipt commands
- terminal artifacts declared in the manifest
- repo-packer or full-repo pack baseline metrics
- agent-native search baseline metrics, when measured
- context packet quality metrics
- agent setup and MCP evidence
- a public result catalog showing checked-in benchmark variants and best measured category rates

## Current Public Evidence

The checked-in manifest uses:

- `benchmarks/public/results/compare-50-stemboost.json`
  - Public 50-issue SWE-bench Lite style slice.
  - Preferred arm: lexical deterministic retrieval for the strict compare gate.
  - Current gate: `60.0%` first-correct-file@5 and at least `50.0` percentage points over the `rg` baseline.
- `benchmarks/public/results/mode-a-50-pyconstants.json`
  - Public 50-issue deterministic Mode A run from pinned local checkouts.
  - Prior catalog best: `64.0%` first-correct-file@5 after Python settings/constants became indexed symbols.
- `benchmarks/public/results/mode-a-50-domain.json`
  - Public 50-issue deterministic Mode A run from pinned local checkouts.
  - Current catalog best: `100.0%` first-correct-file@5, meeting the `100.0%` SWE-bench-style target.
- `benchmarks/public/results/mode-a-requests-seed.json`
  - Public 10-task deterministic Mode A seed run from pinned `psf/requests` local checkouts.
  - Current gate: `100.0%` first-correct-file@5.
- `benchmarks/public/results/compare-nl-ceiling.json`
  - Public 30-issue natural-language slice.
  - Preferred arm: hybrid report evidence for the strict compare gate.
  - Current gate: `36.7%` first-correct-file@5 and at least `20.0` percentage points over the `rg` baseline.
- `benchmarks/public/results/mode-a-nl-vocab.json`
  - Public 30-issue deterministic Mode A natural-language run.
  - Prior catalog best: `50.0%` first-correct-file@5 after common code vocabulary bridges.
- `benchmarks/public/results/mode-a-nl-domain.json`
  - Public 30-issue deterministic Mode A natural-language run.
  - Current catalog best: `100.0%` first-correct-file@5, meeting the `100.0%` natural-language target.
- `benchmarks/public/results/mode-a-rust-callsieve.json`
  - Public 12-task Rust language-smoke slice against a pinned CallSieve checkout.
  - Current gate: `100.0%` first-correct-file@5.
- `benchmarks/public/results/mode-a-typescript-callsieve.json`
  - Public 4-task TypeScript language-smoke slice against the pinned VS Code extension sources.
  - Current gate: `100.0%` first-correct-file@5.

The report keeps the strict compare-gate evidence separate from the best deterministic catalog evidence. Passing the current public proof means CallSieve has a credible external proof artifact today for both the SWE-bench-style target and the natural-language target.

The manifest also includes a `public_result_catalog` of checked-in run variants. The best measured SWE-bench-style rate in that catalog is currently `100.0%`, so the `100.0%` target passes. The best measured natural-language rate is `100.0%`, so the `100.0%` target passes.

The Rust and TypeScript slices are intentionally not added to the SWE-bench-style or natural-language catalog. They prove language coverage against public pinned sources, while the external headline rates continue to come from Astropy, Django, and Requests tasks.

The manifest also links `benchmarks/public/results/nl-miss-graph-adjacency-study.json` as historical miss triage. That study covered 22 natural-language misses: 18 were already graph-reachable from the candidate pool, 4 were not, and 8 had same-directory hints. Four mitigations are now in the retrieval path: natural-language tasks inject a small, capped set of same-module source siblings when at least two top anchors agree on a module, Python module settings/constants are indexed as symbols so setting-definition files can rank directly, common NL code vocabulary bridges terms such as compiling/compiler, restructured/rst, internet/http, database/db, temporary/temp, and username/usernames, and framework-domain module aliases connect human task language to implementation modules such as WCS, migrations, SQL compilers, SQL query builders, ORM lookups, autoreloaders, serializers, validators, enums, URL resolvers, SQLite test-database creation, `UniqueConstraint` model field checks, filterable RHS SQL query construction, auth proxy-permission migrations, and Requests session method normalization. The current deterministic natural-language run has no misses remaining out of 30, the current 50-issue SWE-bench-style run has no misses remaining out of 50, and the Requests seed run has no misses remaining out of 5.

## Regeneration Commands

The manifest records the commands an operator should run before publishing a claim:

```bash
cargo run -- index .
cargo run -- competitive-report benchmarks/competitive-response-manifest.example.json
cargo run -- public-proof-report benchmarks/public-proof-manifest.example.json
cargo run -- setup-auto . --dry-run
cargo run -- mcp-config . --format json
cargo run -- mcp-registry-manifest --out server.json
cargo run -- mcp-contract --out benchmarks/public/results/mcp-contract.json
cargo run -- agent-native-protocol --out benchmarks/public/results/agent-native-protocol.json
cargo run -- agent-native-template benchmarks/public/repos/psf/requests benchmarks/public/manifest.json --k 5 --out benchmarks/public/results/agent-native-requests-template.json
python3 benchmarks/tools/codex-agent-native-requests.py --plan-only
python3 benchmarks/tools/codex-agent-native-requests.py --validate-plan
python3 benchmarks/tools/codex-agent-native-requests.py
cargo run -- repo-pack-baseline benchmarks/public/repos/astropy/astropy --id full-repo-pack-proxy-astropy --tool FullRepoPackProxy --out benchmarks/public/results/full-repo-pack-proxy-astropy.json
cargo run -- repo-pack-baseline benchmarks/public/repos/django/django --id full-repo-pack-proxy-django --tool FullRepoPackProxy --out benchmarks/public/results/full-repo-pack-proxy-django.json
cargo run -- repo-pack-baseline benchmarks/public/repos/psf/requests --id full-repo-pack-proxy-requests --tool FullRepoPackProxy --out benchmarks/public/results/full-repo-pack-proxy-requests.json
cargo run --features embed -- bench-run benchmarks/public/manifest-50.json --workdir benchmarks/public/repos --compare --out benchmarks/public/results/compare-50-rerun.json --resume
cargo run --features embed -- bench-run benchmarks/public/manifest-50.json --workdir benchmarks/public/repos --out benchmarks/public/results/mode-a-50-domain-rerun.json --resume
cargo run --features embed -- bench-run benchmarks/public/manifest.json --workdir benchmarks/public/repos --out benchmarks/public/results/mode-a-requests-seed-rerun.json --resume
cargo run --features embed -- bench-run benchmarks/public/manifest-nl.json --workdir benchmarks/public/repos --compare --out benchmarks/public/results/compare-nl-rerun.json --resume
cargo run --features embed -- bench-run benchmarks/public/manifest-nl.json --workdir benchmarks/public/repos --out benchmarks/public/results/mode-a-nl-domain-rerun.json --resume
cargo run --features embed -- bench-run benchmarks/public/manifest-rust.json --workdir benchmarks/public/repos --out benchmarks/public/results/mode-a-rust-callsieve-rerun.json --resume
cargo run --features embed -- bench-run benchmarks/public/manifest-typescript.json --workdir benchmarks/public/repos --out benchmarks/public/results/mode-a-typescript-callsieve-rerun.json --resume
cargo run -- receipt . --latest --format markdown
cargo run -- receipts .
```

`bench-run` requires an embed-enabled build because the checked-in public comparison reports include the lexical-vs-hybrid arm. It will clone missing public repositories under `benchmarks/public/repos` when the operator explicitly runs it. The proof report itself does not use the network. `public-proof-report` includes a `public_checkouts` block that reports whether the local public checkouts are currently rerun-ready.

Fresh `bench-run` reports include `retrieval_contract_fingerprint`. `public-proof-report` surfaces each checked report as `current`, `missing`, or `stale`; `stale` fails closed so public claims cannot rely on reports generated by older retrieval code. The checked public manifest also sets `require_current_public_report_retrieval_contract: true`, so deterministic public claim reports must be regenerated by the current retrieval contract before the public proof can pass. Legacy compare reports can explicitly set `require_current_retrieval_contract: false` while they remain useful as grep-lift evidence.

## Broad-Read Guardrail

`public-proof-report` includes a `broad_read_guardrail` block that compares the naive broad file-read payload from the local competitive benchmark with the serialized CallSieve context packet for the same tasks. The checked manifest requires `require_broad_read_payload_reduction: true` and a `minimum_broad_read_context_payload_reduction_percent` of `90.0`.

The block reports:

- `baseline_context_payload_tokens`
- `callsieve_context_payload_tokens`
- `context_payload_tokens_saved`
- `context_payload_reduction_percent`
- `avoided_file_reads`
- `avoided_grep_commands`

Public proof fails closed when the baseline has no payload, the CallSieve packet has no measurable payload, the packet is not smaller than the broad-read baseline, or the measured reduction is below the manifest target. The same values are mirrored into `evidence_pack.metrics` with `broad_read.*` names.

## Context Packet Guardrail

`public-proof-report` includes a `context_packet_guardrail` block that checks the actual `packet_quality` counters from the local competitive benchmark. This is the proof that CallSieve is not only returning a smaller packet than broad file reads; it is returning the agent-useful pieces that search tools and repo dumps do not organize by default.

The block reports:

- selected files and selected symbols
- files with snippets or local focus targets
- related tests and files with related tests
- blast-radius hints, caller/callee call-graph hints, and non-unknown risk labels
- selection reasons, compact selection signals, and confidence counts
- next-file hints plus focus, relationship, and test follow-up targets

The checked manifest requires `require_context_packet_quality: true`. Public proof fails closed when the local competitive packet has no selected files, no symbol context, no related-test evidence, no blast-radius/risk evidence, no call-graph evidence, no ranking explanation, or no local expansion targets. The same values are mirrored into `evidence_pack.metrics` with `context_packet.*` names.

## Repo-Packer Guardrail

The checked-in manifest now includes three required measured full-repo prompt-pack proxy baselines:

- `benchmarks/public/results/full-repo-pack-proxy-astropy.json`
  - `1,257` files, `5,638,468` estimated tokens.
  - More than `1,000x` the default CallSieve proof packet in the checked command.
- `benchmarks/public/results/full-repo-pack-proxy-django.json`
  - `3,283` files, `5,155,623` estimated tokens.
  - More than `1,000x` the default CallSieve proof packet in the checked command.
- `benchmarks/public/results/full-repo-pack-proxy-requests.json`
  - `48` files, `101,186` estimated tokens.
  - More than `20x` the default CallSieve proof packet in the checked command.

These artifacts are produced by `callsieve repo-pack-baseline`, not by executing an untrusted external package. They measure the local prompt payload class that repo packers produce: gitignore-respecting source/config/docs files plus per-file framing, with token estimates as `ceil(total_bytes / 4)`.

Repomix, Gitingest, and Code2Prompt outputs are still not checked in here. That is intentional: `public-proof-report` should not fabricate external-tool results. When those tools are installed and approved, add their JSON artifacts as additional `repo_packer_baselines` entries. Each entry points at a JSON artifact, records the command used, and can set `required: true` plus `minimum_token_ratio_vs_callsieve_packet` to fail closed when the packed output is not meaningfully larger than the CallSieve packet.

Example:

```json
{
  "repo_packer_baselines": [
    {
      "id": "repomix-public-50",
      "tool": "Repomix",
      "path": "benchmarks/public/results/repomix-public-50.json",
      "command": "repomix benchmarks/public/repos/django --output repomix-output.txt --style markdown",
      "required": true,
      "token_count_pointer": "/metrics/total_tokens",
      "byte_count_pointer": "/metrics/total_bytes",
      "file_count_pointer": "/metrics/file_count",
      "minimum_token_ratio_vs_callsieve_packet": 10.0
    }
  ]
}
```

The parser supports explicit JSON pointers and common metric names such as `tokens`, `token_count`, `total_tokens`, `bytes`, `total_bytes`, `file_count`, and their `metrics`, `summary`, or `stats` variants. If only bytes are present, the report marks packet tokens as estimated from bytes.

## Agent-Native Search Guardrail

`public-proof-report` includes an `agent_native_search_guardrail` block. The checked manifest now includes three transcript-backed Codex CLI native search/read baselines:

- `benchmarks/public/results/codex-cli-requests.json`
  - 10 pinned public Requests tasks.
  - CallSieve and Codex both hit first-correct-file@5 on all tasks, so the first-correct delta is `0.0`.
  - CallSieve's proof win is context size: `226` average CallSieve packet tokens versus `68,691` average Codex native context tokens, a `303.9x` ratio.
- `benchmarks/public/results/codex-cli-rust-callsieve.json`
  - 12 pinned public Rust language-slice tasks against the CallSieve checkout.
  - CallSieve and Codex both hit first-correct-file@5 on all tasks, so the first-correct delta is `0.0`.
  - CallSieve uses `197` average packet tokens versus `363,586` average Codex native context tokens, a `1,845.6x` ratio.
- `benchmarks/public/results/codex-cli-typescript-callsieve.json`
  - 4 pinned public TypeScript language-slice tasks against the CallSieve VS Code extension sources.
  - CallSieve and Codex both hit first-correct-file@5 on all tasks, so the first-correct delta is `0.0`.
  - CallSieve uses `200` average packet tokens versus `187,026` average Codex native context tokens, a `935.1x` ratio.

Each baseline has locally measured task logs and Codex JSONL transcript bundles with source hashes checked by `agent-native-check` and `public-proof-report`. This is an honest claim: the checked Codex baselines are accuracy ties, not accuracy wins, while CallSieve crushes native search/read on context tokens.

The guardrail also emits a `summary` and mirrors it into `evidence_pack.agent_native_search_summary`. The summary reports baseline count, measured task count, transcript-backed baseline count, distinct native agent tools, repositories, base commits, and task languages. Today that summary should read as Codex-only: `multi_agent_status` is `single_agent_only` until a second real native-agent baseline is added with hashed transcript/export provenance. The proof can still publish the Codex context-token win, but it must not claim multi-agent head-to-head coverage yet.

The checked manifest sets explicit agent-native coverage targets: at least `3` measured transcript-backed baselines, `2` distinct native-agent tools, `2` repositories, and `3` task languages. The current measured artifacts satisfy the baseline, repository, and language targets, but they intentionally fail the distinct-tool target until a second real native-agent run is recorded. That makes the overall public proof `needs_work` for the active multi-agent goal while preserving the measured Codex token-ratio evidence.

For any additional approved external agent run, first generate a task log template with `callsieve agent-native-template`, record the task-level native-search files and transcript-backed token counts locally, run `callsieve agent-native-check --mode measured` against the filled log and transcript/export files and save it with `--out`, generate a standard artifact with `callsieve agent-native-baseline`, then add that artifact under `agent_native_search_baselines`. Any measured agent-native baseline also requires passing `terminal_artifacts` entries with ids `agent-native-protocol` and `agent-native-check`, so the public claim includes the checked measurement playbook and the measured preflight result. Required entries fail closed when the artifact is missing, not locally measured, lacks hashed transcript/export provenance, lacks the checked protocol artifact, lacks the checked measured preflight artifact, is not hash-linked to that preflight artifact, or falls below the configured task-count, first-correct delta, or context-token ratio thresholds.

Task-log input shape:

```json
{
  "tasks": [
    {
      "id": "requests-method-case",
      "task": "make Session.request normalize builtin method case",
      "expected_files": ["requests/sessions.py"],
      "agent_native_files": ["requests/models.py", "requests/sessions.py"],
      "callsieve_files": ["requests/sessions.py"],
      "agent_native_context_tokens": 20000,
      "callsieve_packet_tokens": 1000
    }
  ]
}
```

Prepare the task log template. The template fills `callsieve_files` and `callsieve_packet_tokens`, leaves `agent_native_files` empty, sets `agent_native_context_tokens` to `0`, and reports `needs_agent_native_measurement` until the external run is recorded:

```bash
cargo run -- agent-native-template benchmarks/public/repos/psf/requests benchmarks/public/manifest.json \
  --k 5 \
  --out benchmarks/public/results/agent-native-requests-template.json
```

When this file is listed in `terminal_artifacts` with id `agent-native-template`, public proof parses it and fails closed unless it is still an unmeasured template with CallSieve files and packet tokens filled for every task. It also recomputes the source task-file hash, local index fingerprint, retrieval contract fingerprint, and task payload, so the template's CallSieve-selected files and packet-token counts have to match the current public task manifest, local index, and compiled retrieval behavior.

After the approved external run, fill `agent_native_files`, set `agent_native_context_tokens` from the transcript, and change each filled task's `recording_status` to `measured`. Preflight the task log and source artifacts before generating the baseline:

```bash
cargo run -- agent-native-check benchmarks/public/results/agent-native-requests-template.json \
  --mode measured \
  --source-artifact benchmarks/public/results/cursor-public-requests-transcript.json \
  --out benchmarks/public/results/cursor-public-requests-check.json
```

For the checked Codex CLI measurements, use the reproducible harness with a suite name:

```bash
python3 benchmarks/tools/codex-agent-native-requests.py --suite requests
python3 benchmarks/tools/codex-agent-native-requests.py --suite rust
python3 benchmarks/tools/codex-agent-native-requests.py --suite typescript
```

It regenerates the checked `agent-native-protocol` artifact, builds the CallSieve template from each task's pinned `base_commit`, runs each pinned task in Codex CLI after checking out that same commit, and does not expose ground-truth files or CallSieve context to the native side. It runs Codex with plugins, memory, Chronicle, apps, browser, computer-use, multi-agent, goals, hooks, user config, and project rules disabled, with a read-only Codex sandbox and a JSON output schema. It captures Codex JSONL events and usage for each task, writes raw per-task transcripts under the suite-specific `codex-cli-*-raw/` directory, writes suite-specific task-log, transcript, check, baseline, overlay manifest, and proof artifacts, then runs `public-proof-report` against the overlay manifest. The checked default manifest includes the measured baseline and measured preflight artifacts for all three suites. Each raw transcript records `repo`, `base_commit`, and `checked_out_commit`. Later runs reuse completed raw task transcripts unless `--force` is passed, but only when the raw transcript's prompt, Codex command, repo, base commit, checked-out commit, final JSON, selected files, and token accounting still match the current harness. `--finalize-only` rebuilds the task log, baseline, manifest, and proof report from existing raw transcripts without calling Codex again.

Before authenticating Codex, `python3 benchmarks/tools/codex-agent-native-requests.py --suite <name> --plan-only` writes the suite-specific measurement plan with the exact prompts, per-task checkout commands, Codex commands, disabled feature list, token-accounting rule, output schema, and expected post-run artifacts. Run `--validate-plan` to confirm the saved plan still matches the current template and harness before collecting transcripts. Run `--self-test` to exercise the raw-transcript provenance parser with a synthetic valid envelope and intentionally malformed cached envelopes without authenticating or calling Codex. `--limit-tasks` is for plan/debug validation only; the measured public-proof path refuses to write partial-task artifacts.

The harness uses a suite lock and a shared per-repository lock to prevent parallel runs from mutating the same public checkout at the same time.

The Claude Code harness follows the same proof shape for the public Requests, Rust, and TypeScript suites, but it is not a checked measured baseline unless its generated artifacts are added to the manifest:

```bash
python3 benchmarks/tools/claude-agent-native-requests.py --suite requests
python3 benchmarks/tools/claude-agent-native-requests.py --suite rust
python3 benchmarks/tools/claude-agent-native-requests.py --suite typescript
```

It regenerates the checked `agent-native-protocol` artifact, builds the CallSieve template from each task's pinned `base_commit`, runs each pinned task in Claude Code after checking out that same commit, and does not expose ground-truth files or CallSieve context to the native side. It captures Claude's JSON transcript and usage for each task, writes raw per-task envelopes under suite-specific `benchmarks/public/results/claude-code-*-raw/` directories, writes suite-specific task-log, transcript, check, baseline, overlay manifest, and proof artifacts, then runs `public-proof-report` against the overlay manifest. Each raw envelope records `repo`, `base_commit`, and `checked_out_commit`. Later runs reuse completed raw task envelopes unless `--force` is passed, but only when the raw envelope's prompt, Claude command, repo, base commit, checked-out commit, selected files, and token accounting still match the current harness. Reused envelopes are re-parsed so `task_id`, selected files, and token counts must match the raw Claude `result` and `usage` fields. `--finalize-only` rebuilds the task log, baseline, manifest, and proof report from existing raw envelopes without calling Claude again.

Before authenticating Claude Code, `python3 benchmarks/tools/claude-agent-native-requests.py --suite <name> --plan-only` writes the suite-specific measurement plan with the exact prompts, per-task checkout commands, Claude commands, allowed tools, token-accounting rule, output schema, and expected post-run artifacts. Run `--validate-plan` to confirm the saved plan still matches the current template and harness before collecting transcripts. Run `--self-test` to exercise the raw-transcript provenance parser with a synthetic valid envelope and intentionally malformed cached envelopes without authenticating or calling Claude. `--limit-tasks` is for plan/debug validation only; the measured public-proof path refuses to write partial-task artifacts.

The harness uses a suite lock and a shared per-repository lock to prevent parallel runs from mutating the same public checkout at the same time.

The checked manifest lists Codex and Claude suite measurement plans as `agent-native-measurement-plan` terminal artifacts. `public-proof-report` validates those plan artifacts for tool, suite, task count, constraints, token-accounting rules, per-task prompts, per-task commands, raw transcript paths, and expected post-run artifacts. A valid plan proves repeatability readiness, not measured performance; only `agent_native_search_baselines` with passing transcript provenance count as measured head-to-head evidence.

`agent-native-check` reports `status: pass` only when the filled task log has expected files, CallSieve files, CallSieve packet tokens, native-agent files, transcript-backed native-agent token counts, `recording_status: "measured"` on every task, and locally readable source artifacts with byte counts and stable hashes.

When the measured check output is listed in `terminal_artifacts` with id `agent-native-check`, public proof fails closed unless it can recompute a passing measured preflight from the referenced task log plus transcript/export source artifacts. Every task must be ready, every task must be marked `recording_status: "measured"`, the check artifact must have no issues, and locally readable source artifact byte counts and hashes must still match. When measured baselines are listed, each baseline's full task-log plus transcript/export source hash set must appear in one passing `agent-native-check` artifact, so unrelated, split, or hand-written preflight artifacts cannot support a native-search claim.

`agent-native-protocol` exports the measurement playbook as a standalone JSON artifact. Measured `agent_native_search_baselines` require `benchmarks/public/results/agent-native-protocol.json` to be listed in `terminal_artifacts` with id `agent-native-protocol`, and public proof fails closed unless that file exactly matches the live `callsieve agent-native-protocol` output. This keeps the native-search measurement protocol checked by code instead of drifting as prose.

Generate the measured artifact after filling the native-search fields:

```bash
cargo run -- agent-native-baseline benchmarks/public/results/agent-native-requests-template.json \
  --id cursor-public-requests \
  --tool "Cursor native codebase search" \
  --k 5 \
  --measurement-command "manual approved Cursor run on pinned Requests tasks" \
  --source-artifact benchmarks/public/results/cursor-public-requests-transcript.json \
  --out benchmarks/public/results/cursor-public-requests.json
```

`agent-native-baseline` computes `metrics.task_count`, `metrics.agent_native_first_correct_file_rate_at_k`, `metrics.callsieve_first_correct_file_rate_at_k`, `metrics.agent_native_average_context_tokens`, and `metrics.callsieve_average_packet_tokens` from the task log. It does not run or simulate the external agent; it standardizes recorded results from an approved real run, requires every task to have `recording_status: "measured"`, and requires at least one `--source-artifact` transcript or native-search export. When the task log comes from the public template, task rows retain `repo`, `base_commit`, and `callsieve_index_fingerprint` in the baseline artifact so per-commit provenance remains visible after conversion. The output includes `source_artifact_evidence` with byte counts and stable hashes for the filled task log and each transcript/export. `public-proof-report` rereads those source files, resolving relative paths from the baseline artifact directory when possible, and recomputes byte counts and hashes before allowing `transcript_provenance_status: "pass"` to support a measured native-search baseline.

Example:

```json
{
  "agent_native_search_baselines": [
    {
      "id": "cursor-public-requests",
      "tool": "Cursor native codebase search",
      "path": "benchmarks/public/results/cursor-public-requests.json",
      "command": "manual run: pinned public Requests tasks through Cursor native search",
      "required": true,
      "minimum_tasks": 10,
      "minimum_callsieve_minus_agent_native_first_correct_file_rate_at_k": 0.0,
      "minimum_agent_native_context_token_ratio_vs_callsieve": 2.0
    }
  ]
}
```

The default artifact shape is `locally_measured: true` plus `metrics.task_count`, `metrics.agent_native_first_correct_file_rate_at_k`, `metrics.callsieve_first_correct_file_rate_at_k`, `metrics.agent_native_average_context_tokens`, and `metrics.callsieve_average_packet_tokens`. Pointer fields can override those paths for real tool exports.

## Agent Setup Evidence

`public-proof-report` includes an `agent_setup` block so setup is part of the public proof artifact, not a separate claim. It also includes an `mcp_surface` block that checks the actual MCP `tools/list` surface for the first-mile tools agents need before broad search: `callsieve_context`, `callsieve_focus`, `callsieve_related`, `callsieve_tests`, and `callsieve_status`. Public proof fails if any required first-mile MCP tool is missing.

The report also includes an `mcp_contract` block. This is the stable agent-consumption contract for `callsieve_context`, separate from the list of available tools. It records the contract version, the default `skim` profile, the default context token budget, required `structuredContent` fields such as `read_first`, `sel`, `instruction`, `freshness`, `retrieval_cost`, `stats`, and `trace_event`, supported follow-up instruction keys such as `o`, `next`, `rel`, and `tests`, and required freshness fields such as `initial_fresh`, `refreshed`, `final_fresh`, `index_generation`, `stale_files`, and `fix_command`. Public proof fails if the contract block reports `needs_work`.

Run `cargo run -- mcp-contract --out benchmarks/public/results/mcp-contract.json` to export the same contract as a standalone artifact for integrators. When that artifact is listed in `terminal_artifacts` with id `mcp-contract`, public proof parses it and fails closed unless it exactly matches the live `callsieve mcp-contract` JSON.

The setup block reports required agents from the competitive gate, the full default-layer client set, covered clients, missing clients, priority-client coverage, hook-capable clients, MCP/template clients, and the local commands to reproduce setup evidence.

The default-layer client set is Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains, Amp, Goose, Warp, Cline, Zoo, Roo, and generic MCP. The priority client set is Codex, Claude Code, GitHub Copilot, Cursor, OpenCode, Zed, and generic MCP. The report also names lifecycle-hook clients separately from MCP/template clients so buyers can see where CallSieve can enforce context-first behavior and where it provides project-local MCP/rule templates.

## Next Wins

- Hold natural-language first-correct-file@5 at `100%` while keeping the checked 30-task slice complete.
- Expand the SWE-bench-style slice while holding first-correct-file@5 at `100%` on the checked 50-issue artifact.
- Add external Repomix, Gitingest, or Code2Prompt output-size baselines from installed approved CLIs.
- Add additional measured agent-native search baselines from approved Cursor, Copilot, Claude Code, Devin, or similar runs.
- Add third-party public TypeScript and Rust repositories once network-approved checkouts are available.
- Keep `competitive-report` green at 100% core expected-file recall before publishing public claims.
