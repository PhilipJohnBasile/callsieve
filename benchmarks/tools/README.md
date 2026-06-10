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

Edit the hardcoded paths (`/tmp/gapstudy`, the binary path) before use.
