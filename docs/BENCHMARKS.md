# Benchmark Evidence

CallSieve benchmark suites are JSON task packs that measure whether the context packet selects the files an agent should read first.

Run the real-repo CallSieve suite:

```bash
cargo run -- index .
cargo run -- benchmark-suite . benchmarks/callsieve-real-repo.json
cargo run -- trace-replay . benchmarks/callsieve-real-repo.json benchmarks/session-trace.local.json --limit 20
cargo run -- trace-summary benchmarks/session-trace.example.json
cargo run -- session-start . "change the read-first context packet ranking" --client codex --model gpt-5-codex --trace .callsieve/observed-session.json
cargo run -- session-event .callsieve/observed-session.json --command "callsieve agent-context . \"change the read-first context packet ranking\"" --tokens 3000 --phase callsieve
cargo run -- session-finish .callsieve/observed-session.json --out .callsieve/observed-summary.json
cargo run -- trace-check benchmarks/session-trace.example.json --strict
cargo run -- benchmark-doctor benchmarks/report-manifest.example.json
cargo run -- benchmark-report benchmarks/report-manifest.example.json
cargo run -- pilot-doctor benchmarks/pilot-manifest.example.json
cargo run -- pilot-report benchmarks/pilot-manifest.example.json
cargo run -- proof-report benchmarks/pilot-manifest.example.json
cargo run -- evidence-pack benchmarks/pilot-manifest.example.json --anonymize
cargo run -- policy-check benchmarks/session-trace.example.json --strict
```

Use `cargo run -- index . --lsp` when benchmark evidence should include local LSP-derived reference edges. Reports stay deterministic and local-first: CallSieve only uses language servers already installed on the machine, and falls back to the tree-sitter/heuristic graph when a server is unavailable.

Latest local run on this repository:

- expected-file recall: `16/16` (`100%`)
- total estimated token savings: `541090`
- average estimated token reduction: `98.6%`
- avoided grep commands: `34`
- avoided file reads: `87`

The suite output includes:

- `expected_file_recall`: fraction of expected files selected by `callsieve context`
- `total_estimated_token_savings`: estimated tokens avoided versus a naive grep/read loop
- `total_estimated_avoided_grep_commands`: estimated grep commands avoided
- `total_estimated_avoided_file_reads`: estimated file reads avoided
- `misses`: per-task missing expected files with likely failure reasons
- `observed_session`: optional aggregate metrics from real agent traces

`callsieve status <repo>` records whether the current index has `"lsp_enriched": true` and which local LSP commands were available during indexing.

## Task Format

```json
{
  "tasks": [
    {
      "id": "context-packet-ranking",
      "task": "change the read-first context packet ranking and selected file output",
      "expected_files": [
        "src/query/mod.rs",
        "src/query/ranker.rs",
        "tests/cli.rs"
      ]
    }
  ]
}
```

## Session Trace Format

Use `session` when you have actual baseline and CallSieve-assisted agent trace numbers. `observed` is still accepted as a backward-compatible alias.

```json
{
  "id": "real-session",
  "task": "change the read-first context packet ranking and selected file output",
  "expected_files": ["src/query/mod.rs"],
  "session": {
    "baseline": {
      "grep_commands": 7,
      "file_reads": 12,
      "tokens": 28000,
      "commands": ["rg context", "rg read_first"],
      "files_read": ["src/query/mod.rs"],
      "notes": ["Trace from an agent session without CallSieve."]
    },
    "callsieve": {
      "grep_commands": 1,
      "file_reads": 5,
      "tokens": 9000,
      "commands": ["callsieve context . \"change the read-first context packet ranking and selected file output\""],
      "files_read": ["src/query/mod.rs"],
      "notes": ["Trace from the same task after calling CallSieve first."]
    }
  }
}
```

There is also a starter file at `benchmarks/session-trace.example.json`.

Summarize standalone traces with:

```bash
cargo run -- trace-summary benchmarks/session-trace.example.json
```

The summary reports sessions, baseline tokens, CallSieve tokens, token savings, token reduction percent, avoided grep commands, avoided file reads, and files still missed by the CallSieve-assisted run.

Generate a controlled local replay trace from a benchmark suite with:

```bash
cargo run -- trace-replay <repo> <suite.json> <trace.json> --limit 20
```

`trace-replay` writes the same `tasks[].session.baseline/callsieve` JSON shape accepted by `trace-summary`, `benchmark-report`, `pilot-report`, and `proof-report`. It is tagged as `metadata.collection = "controlled_replay"` and is not human telemetry. It deterministically simulates the baseline as task-term grep plus full reads of every matched indexed file, then counts CallSieve as the serialized context packet plus full reads of the selected `read_first` files.

For real Codex/ChatGPT observations, use `session-start`, `session-event`, and `session-finish`. These traces are tagged as `metadata.collection = "observed_session"` and store ordered `events[]` with command classification, phase, files read, and optional token counts. Baseline events contribute comparison metrics; CallSieve-phase events are used for strict before-grep policy checks.

Audit before-grep policy with:

```bash
cargo run -- trace-check benchmarks/session-trace.example.json
cargo run -- trace-check benchmarks/session-trace.example.json --strict
cargo run -- policy-check benchmarks/session-trace.example.json --strict
```

`trace-check` fails when an observed CallSieve-assisted session runs `rg`, `grep`, or `ripgrep` before `callsieve context`, `agent-context`, `guard`, `grep`, or the MCP `callsieve_context` tool. With `--strict`, it also fails common file reads such as `cat`, `sed`, `nl`, `Get-Content`, and `read_file` before CallSieve context.

`policy-check` prints the same JSON check result but exits nonzero on violations, which makes it suitable for CI or scripted pilot audits.

## Codex/ChatGPT Sessions

When Codex/ChatGPT is the available agent, start each measured task with `codex-session`:

```bash
cargo run -- codex-session . "change the read-first context packet ranking" --trace-out benchmarks/codex-session.local.json --model gpt-5-codex
cargo run -- trace-summary benchmarks/codex-session.local.json
cargo run -- enforce . --client codex --trace benchmarks/codex-session.local.json --strict
```

`codex-session` returns the normal context packet and writes model-tagged controlled replay JSON. The baseline side is deterministic grep/read replay for the same task; the CallSieve side records a Codex/ChatGPT context-first session start and counts the context packet plus full reads of selected `read_first` files. Use `--expected-file <path>` repeatedly when the task has known expected files. Run the same task with different `--model` values to compare the Codex/ChatGPT models available in the local environment. CallSieve labels and audits the sessions you run; it does not invoke hidden ChatGPT models itself.

Current local Codex/ChatGPT pilot:

```bash
cargo run -- pilot-doctor benchmarks/codex-chatgpt-manifest.local.json
cargo run -- pilot-report benchmarks/codex-chatgpt-manifest.local.json --limit 14
```

- model label: `gpt-5-codex`
- benchmark recall: `16/16` expected files (`100%`)
- benchmark estimate: `378701` tokens saved, `79.8%` token reduction, `34` avoided grep commands, `56` avoided file reads
- Codex/ChatGPT controlled replay trace: `1` session, `122943` baseline tokens, `118350` CallSieve tokens, `4593` tokens saved, `3.7%` token reduction, `12` avoided grep commands, `16` avoided file reads, `0` files still missed
- policy and setup: `pilot-report` pass, `0` strict trace-policy violations, fresh LSP-enriched index

## Multi-Repo Reports

Use `benchmark-report` when you want one compact evidence packet across multiple local repositories. It never clones repositories and never requires network access.

Manifest format:

```json
{
  "repos": [
    {
      "label": "callsieve",
      "path": ".",
      "suite_paths": ["benchmarks/callsieve-real-repo.json"],
      "trace_paths": ["benchmarks/session-trace.example.json"]
    },
    {
      "label": "another-local-repo",
      "path": "../another-local-repo",
      "suite_path": "../another-local-repo/benchmarks/tasks.json"
    }
  ]
}
```

Run:

```bash
cargo run -- benchmark-doctor benchmarks/report-manifest.example.json
cargo run -- benchmark-report benchmarks/report-manifest.example.json
```

`benchmark-doctor` validates local repo paths, indexes, suites, and trace files before collection. The report output includes per-repo expected-file recall, estimated token savings, avoided grep commands, avoided file reads, misses, and aggregate totals across all listed repos.

## External GitHub Fixture Pilot

The local external pilot uses shallow, read-only GitHub clones under ignored `benchmarks/github-*` directories. Do not run dependency installs, build scripts, or tests inside those repos for evidence collection; CallSieve only indexes source files.

Current fixture repos:

- `BurntSushi/ripgrep`
- `sharkdp/fd`
- `tokio-rs/axum`
- `pallets/flask`
- `psf/black`
- `encode/httpx`

Run the local external manifest after the fixture repos have been cloned and indexed:

```bash
cargo run -- benchmark-doctor benchmarks/external-github-manifest.local.json
cargo run -- benchmark-report benchmarks/external-github-manifest.local.json
cargo run -- benchmark-report benchmarks/external-github-manifest.local.json --limit 24
cargo run -- trace-replay benchmarks/github-ripgrep benchmarks/external-ripgrep-suite.json benchmarks/external-ripgrep-trace.json --limit 20
```

Latest local external run:

- 12-file packet: `25/28` expected files (`89.3%` recall), `3363324` estimated tokens saved, `90.7%` estimated token reduction, `45` avoided grep commands, `1115` avoided file reads
- 24-file packet: `28/28` expected files (`100%` recall), `3165185` estimated tokens saved, `83.1%` estimated token reduction, `45` avoided grep commands, `981` avoided file reads
- controlled replay traces at 24-file packet: `12` sessions, `3581345` baseline tokens, `1642220` CallSieve tokens, `1939125` tokens saved, `54.1%` token reduction, `57` avoided grep commands, `1021` avoided file reads, `0` files still missed
- pilot report over the same manifest: `pass`, `0` strict trace-policy violations, `6/6` fresh indexes

This is external repo benchmark evidence plus controlled local replay support, not observed human agent-session proof. Add `trace_paths` from `session-start` / `session-event` / `session-finish` when running a pilot intended to prove session behavior. Use generated `trace-replay` files when you need reproducible local replay traces before real session telemetry exists. The external GitHub manifest should be reported at `--limit 24`; `--limit 12` is useful as a stricter budget diagnostic when investigating ranking quality.

## Pilot Reports

Use `pilot-report` when you need one local JSON artifact for a pilot: benchmark recall, estimated token savings, observed trace savings, controlled replay counts, strict before-grep policy checks, index freshness, daemon state, Codex bootstrap coverage, and LSP coverage.

Use `proof-report` for the top-level claim artifact. It keeps observed sessions, controlled replay sessions, external repo coverage, and observed token reduction separate. Controlled replay traces are never counted as observed evidence, and traces tagged as observed but containing controlled replay markers fail the report.

```bash
cargo run -- index . --lsp
cargo run -- watch . --lsp
cargo run -- pilot-doctor benchmarks/pilot-manifest.example.json
cargo run -- pilot-report benchmarks/pilot-manifest.example.json
cargo run -- proof-report benchmarks/pilot-manifest.example.json
```

Current proof-sprint artifact:

```bash
cargo run -- proof-report benchmarks/proof-sprint-manifest.local.json --limit 8 --no-snippets
```

Latest local proof sprint: `pass`, `1` observed Codex session, `0` controlled replay sessions, `9587` baseline tokens, `1747` CallSieve tokens, `7840` tokens saved, `81.8%` observed token reduction, `0` missed expected files, `0` strict trace-policy violations, fresh LSP index, daemon freshness, and Codex bootstrap present.

Pilot manifests support the same repo entries as `benchmark-report`, plus optional `languages` and thresholds:

```json
{
  "thresholds": {
    "minimum_recall": 1.0,
    "minimum_token_reduction_percent": 50.0,
    "minimum_observed_sessions": 1,
    "minimum_observed_token_reduction_percent": 50.0,
    "minimum_external_repos": 0,
    "maximum_controlled_replay_ratio": 0.25,
    "maximum_trace_violations": 0,
    "require_fresh_index": true,
    "require_lsp_where_available": false,
    "require_codex_bootstrap": false
  },
  "repos": [
    {
      "label": "callsieve",
      "path": ".",
      "languages": ["typescript", "javascript", "python", "rust"],
      "suite_paths": ["benchmarks/callsieve-real-repo.json"],
      "trace_paths": ["benchmarks/session-trace.example.json"]
    }
  ]
}
```

The status is `pass` only when all thresholds pass. Keep `require_lsp_where_available` false when pilot machines may have language servers installed but the index was intentionally built without `--lsp`.

## Evidence Packs

Use `evidence-pack` to package the pilot report into a shareable JSON envelope for external repo/session collection:

```bash
cargo run -- evidence-pack benchmarks/pilot-manifest.example.json --anonymize
```

The output includes generation time, the local collection protocol, and the complete pilot report. With `--anonymize`, repo labels and local paths are redacted while aggregate counts, recall, token savings, trace violations, and LSP coverage remain available.

Recommended external proof target:

- `10-20` real local repos
- `50-100` observed agent sessions
- strict trace policy checks enabled
- indexes built with `--lsp` where local language servers are available
- aggregate JSON published without proprietary code

## Interpreting Misses

`benchmark-suite` reports `misses` when an expected file is not selected. Common reasons:

- the expected file is not currently indexed
- the expected file fell outside `--limit`
- the task wording did not overlap symbol, path, or keyword signals
- the current deterministic graph did not connect the expected file
- selected files had no matching indexed symbols

Start by rerunning with a larger limit:

```bash
cargo run -- benchmark-suite . benchmarks/callsieve-real-repo.json --limit 12
```

If recall improves only by increasing `--limit`, ranking needs work. If recall does not improve, indexing, parsing, or reference extraction needs work.
