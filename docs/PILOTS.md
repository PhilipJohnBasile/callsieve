# CallSieve Pilots

CallSieve's core local CLI and MCP server are open source under the MIT License. The commercial offer is paid adoption and proof work for teams that want to reduce agent grep, repeated reads, and context waste without sending proprietary code to a cloud service.

## What Is Open Source

The public repo includes:

- local repository indexing
- deterministic `agent-context` retrieval
- MCP tools for agent integration
- context-first policy checks
- benchmark and retrieval-eval commands
- trace, pilot, proof-report, and evidence-pack workflows
- docs and tests needed to run CallSieve locally

The MIT license allows commercial use of the code. It does not by itself provide paid support, private deployment work, custom integration, or claim validation for a team's internal workflows.

## What A Paid Pilot Covers

A paid pilot should sell the outcome, not access to the source code:

- install CallSieve locally on selected repositories
- configure Codex, Claude, Cursor, Cline, Roo, or a generic MCP client
- establish the first-command policy: `callsieve agent-context <repo> "<task>"`
- record paired baseline and CallSieve-assisted sessions
- measure grep commands, file reads, selected files, critical misses, and token counts
- produce a shareable evidence pack with anonymized aggregate metrics when needed
- identify retrieval misses and tune fixtures, docs, and setup
- hand off a repeatable local workflow that does not require cloud services or API keys

## Pilot Success Criteria

Use concrete gates instead of broad language:

- `pilot-qa` passes for claim-counted sessions
- `proof-report` passes for the pilot manifest
- strict trace-policy violations are `0`
- critical-file misses are `0`
- observed token reduction is measured from real paired transcript context token counts
- `context_payload_reduction` is described as an estimate, not observed whole-session savings

Use `enterprise-proof-report` only for broad developer-session claims. Until that report passes, avoid phrases like "almost all developer sessions" or broad enterprise-scale guarantees.

## Evidence Flow

```bash
callsieve index <repo> --lsp
callsieve agent-context <repo> "<task>"
callsieve pilot-init benchmarks/evidence/pilot.local.json --sessions 100
callsieve pilot-task add benchmarks/evidence/pilot.local.json <repo> "<task>" --id <id> --expected-file <path> --critical-file <path>
callsieve pilot-run benchmarks/evidence/pilot.local.json --task-id <id> --mode baseline --command "<baseline command>" --files-read <path> --tokens <n>
callsieve pilot-run benchmarks/evidence/pilot.local.json --task-id <id> --mode callsieve --command "callsieve agent-context <repo> \"<task>\"" --files-read <path> --tokens <n>
callsieve pilot-qa benchmarks/evidence/pilot.local.json
callsieve pilot-finalize benchmarks/evidence/pilot.local.json --out benchmarks/evidence/proof.local.json
callsieve evidence-pack benchmarks/evidence/pilot.local.json --anonymize
```

## Positioning

CallSieve should be sold as a local-first codebase filter for AI coding agents. The direct value proposition is making agents read the right small set of files before they spend context on blind grep, repo-wide search, and repeated file reads.

The strongest pilot question is:

Can this team make its agents read the right 5 files instead of grepping through 50?
