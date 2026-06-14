# Benchmark tooling

Ad-hoc analysis harnesses used to design retrieval mechanisms. They expect
isolated clones (never the live bench workdirs) and a worklist TSV of
`issue_id<TAB>repo<TAB>base_commit<TAB>truth_path` plus a `tasks.tsv` of
`issue_id<TAB>base64(task)` derived from a public manifest.

- `nl-miss-graph-adjacency-study.py` — for each missed issue: checkout, build a
  lexical index, take the 8-candidate pool, and report whether the ground-truth
  file is one import/reference/same-directory hop away. Produced
  `results/nl-miss-graph-adjacency-study.json` (82% reachable), the evidence
  behind the graph-consensus ranking signal.
- `nl-miss-hitcount.py` — same loop, but just counts ground-truth top-5 hits
  with the currently built `target/release/callsieve`; the fast iteration
  harness for ranking changes (one run ≈ 15–20 min for 22 issues).
- `codex-agent-native-requests.py`: runs the checked transcript-backed Codex
  CLI native search/read baselines for `--suite requests`, `--suite rust`, and
  `--suite typescript`. It keeps ground-truth files and CallSieve context out
  of Codex prompts, disables Codex plugins, memory, Chronicle, apps, browser,
  computer-use, multi-agent, goals, hooks, user config, and project rules, runs
  Codex in a read-only sandbox with a JSON output schema, captures Codex JSONL
  transcripts and usage, fills suite-specific task logs, runs
  `agent-native-check`, `agent-native-baseline`, writes overlay public-proof
  manifests, and saves resulting proof reports. Raw per-task transcripts are
  saved in suite-specific `codex-cli-*-raw/` directories and reused on later
  runs unless `--force` is passed, but only when their prompt, command, repo,
  `base_commit`, checked-out commit, final JSON, selected files, and token
  accounting still match the current harness. Use `--plan-only`,
  `--validate-plan`, and `--self-test` before collecting transcripts. The
  harness uses both suite locks and shared per-repository locks so Rust and
  TypeScript runs cannot mutate the same public checkout in parallel.
- `claude-agent-native-requests.py` — runs transcript-backed Claude Code
  native-search baselines for `--suite requests`, `--suite rust`, and
  `--suite typescript`. It refuses to run real tasks unless `claude auth status`
  is logged in, keeps ground-truth files out of Claude prompts, regenerates the
  checked `agent-native-protocol` artifact, builds the CallSieve template from
  each task's pinned `base_commit`, checks out that same commit before each
  Claude task, captures Claude JSON transcripts, fills suite-specific task
  logs, runs `agent-native-check`, `agent-native-baseline`, writes an overlay
  public-proof manifest, and saves the resulting proof report. Raw per-task
  Claude JSON envelopes are saved in suite-specific `claude-code-*-raw/`
  directories and reused on later runs unless `--force` is passed, but only when
  their prompt and command still match the current harness and their `repo`,
  `base_commit`, and `checked_out_commit` match the task. Reused envelopes are
  re-parsed so `task_id`, selected files, and token counts must match the raw
  Claude `result` and `usage` fields. Use `--finalize-only` after a completed or
  partially resumed measurement to rebuild the task log, baseline, overlay
  manifest, and proof report without calling Claude again. Use `--plan-only`,
  `--validate-plan`, and `--self-test` before collecting transcripts.
  `--limit-tasks` is only for plan/debug validation; measured public-proof
  artifacts always cover the full selected suite. The harness writes both a
  suite lock and a shared per-repository lock so Rust and TypeScript runs cannot
  mutate the same public checkout in parallel.

Edit the hardcoded paths (`/tmp/gapstudy`, the binary path) before use.
