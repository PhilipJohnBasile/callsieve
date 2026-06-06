# CallSieve Pilots

CallSieve's core local CLI and MCP server are open source under the MIT License. The commercial offer is paid adoption and proof work for teams that want to reduce agent grep, repeated reads, and context waste without sending proprietary code to a cloud service.

The retrieval step itself spends zero AI model tokens because CallSieve ranks locally. Pilot reports must still account for the context packet and all later agent session tokens separately.

## What Is Open Source

The public repo includes:

- local repository indexing
- deterministic `agent-context` retrieval
- opt-in local hybrid retrieval with `--embeddings`
- CODEOWNERS, git activity, and stack-trace context signals
- MCP tools for agent integration
- context-first policy checks
- benchmark and retrieval-eval commands
- trace, pilot, proof-report, and evidence-pack workflows
- docs and tests needed to run CallSieve locally

The MIT license allows commercial use of the code. It does not by itself provide paid support, private deployment work, custom integration, or claim validation for a team's internal workflows.

## What A Paid Pilot Covers

A paid pilot should sell the outcome, not access to the source code:

- install CallSieve locally on selected repositories
- configure Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, Cline, Zoo Code, the deprecated Roo alias, or a generic MCP client
- establish the first-command policy: `callsieve agent-context <repo> "<task>"`
- explain the token boundary: zero AI model tokens for local retrieval, measured context tokens for returned packets and sessions
- record paired baseline and CallSieve-assisted sessions
- measure grep commands, file reads, selected files, critical misses, and token counts
- produce a shareable evidence pack with anonymized aggregate metrics when needed
- identify retrieval misses and tune fixtures, docs, and setup
- evaluate optional flags such as `--embeddings`, `--git-boost`, and `--error` against the team's own benchmark tasks before changing workflow defaults
- hand off a repeatable local workflow that does not require cloud services or API keys

## Pilot Success Criteria

Use concrete gates instead of broad language:

- `pilot-qa` passes for claim-counted sessions
- `proof-report` passes for the pilot manifest
- strict trace-policy violations are `0`
- critical-file misses are `0`
- observed token reduction is measured from real paired transcript context token counts
- `context_payload_reduction` is described as an estimate, not observed whole-session savings
- `retrieval_cost.retrieval_model_tokens = 0` is described as retrieval-only, not whole-session savings
- hybrid retrieval is described as opt-in unless a measured pilot explicitly enables it

Use `enterprise-proof-report` only for broad developer-session claims. Until that report passes, avoid phrases like "almost all developer sessions" or broad enterprise-scale guarantees.

## Claude Proof Sprint

Use `proof-sprint` when a pilot buyer wants the fastest credible observed proof path with Claude Code:

```bash
callsieve proof-sprint init benchmarks/evidence/proof-sprint.local.json --client claude --sessions 10 --model claude-opus-4-8
callsieve proof-sprint status benchmarks/evidence/proof-sprint.local.json
callsieve proof-sprint run benchmarks/evidence/proof-sprint.local.json --resume
callsieve proof-sprint finalize benchmarks/evidence/proof-sprint.local.json --out benchmarks/evidence/proof.local.json
```

`proof-sprint status` is the operator dashboard. It reports paired sessions complete, missing baseline and CallSieve phases, observed token reduction when transcripts exist, critical misses, strict trace violations, transcript-accounting coverage, QA status, and the next command to run. `proof-sprint run --resume` collects the next missing phase, preferring the CallSieve half of a partially collected pair before starting a new baseline. Use `--dry-run` to print the next Claude collection plan without spawning Claude, and `--limit N` to collect more than one phase in a single invocation.

The 10-session sprint is useful for paid-pilot qualification. The 50-session target remains the first serious public claim gate. Both paths use the same observed-session manifest, `pilot-qa`, and `proof-report` gates.

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
