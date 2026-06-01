# Benchmark Evidence

CallSieve benchmark suites are JSON task packs that measure whether the context packet selects the files an agent should read first.

Run the real-repo CallSieve suite:

```bash
cargo run -- index .
cargo run -- benchmark-suite . benchmarks/callsieve-real-repo.json
```

The suite output includes:

- `expected_file_recall`: fraction of expected files selected by `callsieve context`
- `total_estimated_token_savings`: estimated tokens avoided versus a naive grep/read loop
- `total_estimated_avoided_grep_commands`: estimated grep commands avoided
- `total_estimated_avoided_file_reads`: estimated file reads avoided
- `misses`: per-task missing expected files with likely failure reasons
- `observed_session`: optional aggregate metrics from real agent traces

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
