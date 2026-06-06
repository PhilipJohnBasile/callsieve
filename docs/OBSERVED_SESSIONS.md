# Observed Session Protocol

This protocol is for proving observed whole-session token savings across real developer sessions.

Use this protocol when making claims about real token savings. Do not use benchmark estimates, controlled replay, local rehearsal, or `context_payload_reduction` as observed whole-session proof.

CallSieve retrieval itself has `retrieval_cost.retrieval_model_tokens = 0` because ranking runs locally. Observed-session claims still require transcript token accounting for the returned packet, file reads, reasoning, tool output, and final answer.

## Claim Gate

The first serious public claim should be:

```text
In 50 paired observed Claude Code or Codex developer sessions across six OSS repos, CallSieve reduced transcript context tokens by at least 50% with zero critical misses and zero strict before-grep policy violations.
```

Do not claim broad developer-session coverage until `enterprise-proof-report` passes. The enterprise gate requires 1,000 paired observed sessions, multi-client coverage, multiple languages, scale-proxy repos, strict trace policy, and full transcript token accounting.

## What Counts

A claim-counted session must have:

- a preregistered task id, repo, task text, expected files, and critical files
- one baseline phase without CallSieve-first context
- one CallSieve phase using `callsieve agent-context`, MCP `callsieve_context`, `begin`, `guard`, or hook launcher context first
- the CallSieve phase records local retrieval as zero AI model tokens only when the output includes `retrieval_cost.retrieval_model_tokens = 0`
- transcript context token counts copied from the real agent transcript, platform UI, or audited Claude Code JSON usage output
- proof-mode traces created with `begin --proof-trace` must add later events with `--tokens` and explicit `--phase baseline|callsieve`
- `files_read` values are files actually read by the agent, copied from the transcript or tool log
- for CallSieve phases, files selected into the compact context packet recorded as `context_selected_files` when the agent can answer from context without whole-file reads
- exact command or tool summary for each phase
- `metadata.collection = "observed_session"`
- lifecycle hook collections such as `codex_hook_trace`, `claude_hook_trace`, and `<client>_hook_trace` are guardrail telemetry and are excluded from observed proof gates
- `token_accounting.source = "transcript_context_tokens"`
- `pilot-qa` passing before `pilot-finalize` or `proof-report`

For Claude Code JSON, CallSieve records:

```text
usage.input_tokens + usage.cache_creation_input_tokens + usage.cache_read_input_tokens + usage.output_tokens
```

The recorder stores that total as `transcript_context_tokens` and attaches the raw `--usage-json` path plus the token breakdown to trace event `token_evidence`. It accepts Claude Code `json` or `stream-json` artifacts. Prefer `stream-json --verbose` because CallSieve can extract `Read` tool file paths from the same transcript artifact.

A session does not count if:

- token counts are estimated
- file reads are guessed
- the task was not preregistered
- baseline and CallSieve phases were mixed into one contaminated transcript
- the operator learned the answer in one phase and reused that knowledge in the other phase
- controlled replay, benchmark output, or local-model rehearsal is labeled as observed evidence
- the CallSieve phase grepped or read broad files before context
- any critical file was missed

Reject invalid runs instead of deleting them:

```bash
callsieve pilot-task reject <manifest.json> --task-id <task-id> --reason "<audit reason>"
```

## Setup

For a Claude Code paid-pilot proof sprint, use the wrapper command group:

```bash
callsieve proof-sprint init benchmarks/evidence/proof-sprint.local.json --client claude --sessions 10 --model claude-opus-4-8
callsieve proof-sprint status benchmarks/evidence/proof-sprint.local.json
callsieve proof-sprint run benchmarks/evidence/proof-sprint.local.json --resume
```

This preregisters the same OSS task matrix as the 50-session Claude milestone, but sets the observed-session target to `10`. Use `--sessions 50` for the first public claim gate.

Create a 50-session milestone manifest for the agent you are observing:

```bash
callsieve setup-observed-codex-oss-50
callsieve setup-observed-claude-oss-50 --model claude-opus-4-8
```

This writes an ignored local manifest:

```text
benchmarks/evidence/observed-codex-oss-50.local.json
benchmarks/evidence/observed-claude-oss-50.local.json
```

The manifest preregisters 50 tasks across the external OSS fixture repos and sets:

- `minimum_observed_sessions = 50`
- `minimum_observed_token_reduction_percent = 50.0`
- `require_transcript_token_accounting = true`
- `token_accounting_source = transcript_context_tokens`
- expected files copied into critical files

Run QA before collection. It should fail until real paired sessions are recorded:

```bash
callsieve pilot-qa benchmarks/evidence/observed-codex-oss-50.local.json
callsieve pilot-qa benchmarks/evidence/observed-claude-oss-50.local.json
callsieve proof-sprint status benchmarks/evidence/proof-sprint.local.json
```

## Run A Task

For each task, record two separate real transcript phases.

Baseline phase:

1. Start a fresh agent transcript.
2. Give the task without telling the agent to use CallSieve first.
3. Let the agent use its normal repo search workflow.
4. Record the exact transcript context token count from the platform.
5. Record every file the agent actually read.
6. Record the command/tool summary that best represents the phase.

CallSieve phase:

1. Start a fresh agent transcript.
2. Require CallSieve context before broad search.
3. Use one of:

```bash
callsieve agent-context <repo> "<task>"
callsieve begin <repo> "<task>" --client codex --trace-out <trace.json> --proof-trace
callsieve hook install <repo> --client codex --strict --force --lsp
```

4. Let the agent solve the task.
5. Record the exact transcript context token count from the platform.
6. Record every file the agent actually read, and record CallSieve-selected context files separately if the agent did not need whole-file reads.
7. Record the command/tool summary that best represents the phase.

When optional retrieval flags are used, include them in the recorded command summary. For example, record `--embeddings`, `--git-boost`, and `--error <trace>` when those flags are part of the CallSieve phase. Do not compare a baseline against an undocumented retrieval configuration.

Record the observed phases with the helper:

```bash
callsieve record-codex-observed-session \
  --manifest benchmarks/evidence/observed-codex-oss-50.local.json \
  --task-id <task-id> \
  --mode baseline \
  --command "<baseline command or tool summary>" \
  --tokens <real-transcript-context-tokens> \
  --files-read <file> \
  --files-read <file>

callsieve record-codex-observed-session \
  --manifest benchmarks/evidence/observed-codex-oss-50.local.json \
  --task-id <task-id> \
  --mode callsieve \
  --command "callsieve agent-context <repo> \"<task>\"" \
  --tokens <real-transcript-context-tokens> \
  --files-read <file> \
  --files-read <file>
```

For Claude Code, save the stream JSON result and record with the generic helper. If the stream contains `Read` tool calls, `--files-read` can be omitted because CallSieve extracts them from the artifact.

```bash
callsieve collect-claude-observed-session \
  --manifest benchmarks/evidence/observed-claude-oss-50.local.json \
  --task-id <task-id> \
  --mode baseline \
  --max-budget-usd 2.00

callsieve collect-claude-observed-session \
  --manifest benchmarks/evidence/observed-claude-oss-50.local.json \
  --task-id <task-id> \
  --mode callsieve \
  --context-limit 4 \
  --snippets-per-file 1 \
  --max-budget-usd 2.00
```

The collector is preferred when available because it spawns Claude Code directly, saves `.callsieve/observed-<task-id>-<mode>.ndjson`, extracts `Read` tool calls, records `context_selected_files` from the CallSieve packet, and records the phase. The default CallSieve phase uses a compact 4-file, one-snippet context packet so the proof can measure whether selected context avoids whole-file reads.

Manual equivalent:

```bash
printf "%s" "<baseline task prompt>" | claude -p --input-format text --output-format stream-json --verbose --no-session-persistence --max-budget-usd 2.00 > .callsieve/observed/<task-id>-baseline.ndjson

callsieve record-observed-session \
  --manifest benchmarks/evidence/observed-claude-oss-50.local.json \
  --client claude \
  --model claude-opus-4-8 \
  --task-id <task-id> \
  --mode baseline \
  --command "claude -p --input-format text <baseline prompt on stdin> --output-format stream-json --verbose" \
  --usage-json .callsieve/observed/<task-id>-baseline.ndjson

callsieve agent-context <repo> "<task>" --format markdown > .callsieve/observed/<task-id>-callsieve-context.md
printf "%s" "<task prompt with CallSieve context first>" | claude -p --input-format text --output-format stream-json --verbose --no-session-persistence --max-budget-usd 2.00 > .callsieve/observed/<task-id>-callsieve.ndjson

callsieve record-observed-session \
  --manifest benchmarks/evidence/observed-claude-oss-50.local.json \
  --client claude \
  --model claude-opus-4-8 \
  --task-id <task-id> \
  --mode callsieve \
  --command "callsieve agent-context <repo> \"<task>\" && claude -p --input-format text <CallSieve prompt on stdin> --output-format stream-json --verbose" \
  --usage-json .callsieve/observed/<task-id>-callsieve.ndjson \
  --context-selected-file <file selected by CallSieve>
```

Run QA after each pair:

```bash
callsieve pilot-qa benchmarks/evidence/observed-codex-oss-50.local.json
callsieve pilot-qa benchmarks/evidence/observed-claude-oss-50.local.json
```

## Finalize Proof

After 50 countable paired sessions:

```bash
callsieve pilot-qa benchmarks/evidence/observed-codex-oss-50.local.json
callsieve pilot-finalize benchmarks/evidence/observed-codex-oss-50.local.json --out benchmarks/evidence/observed-codex-oss-50-proof.local.json --limit 24
callsieve proof-report benchmarks/evidence/observed-codex-oss-50-proof.local.manifest.json --limit 24
```

Use the matching Claude manifest and output paths for Claude Code collection.

The claim is usable only if:

- `pilot-qa` status is `pass`
- `proof-report` status is `pass`
- observed sessions are at least 50
- observed token reduction is at least 50%
- transcript token accounting is 100%
- strict trace policy violations are zero
- critical files still missed are zero
- controlled replay is reported separately and not counted as observed

## Operator Log Template

Use this for each task:

```text
task_id:
repo:
task:
client:
model:
operator:
date:

baseline_transcript_url_or_id:
baseline_transcript_context_tokens:
baseline_usage_json:
baseline_command_summary:
baseline_files_read:

callsieve_transcript_url_or_id:
callsieve_transcript_context_tokens:
callsieve_usage_json:
callsieve_command_summary:
callsieve_files_read:
callsieve_context_selected_files:

critical_files_found:
critical_files_missed:
reject_reason_if_any:
```

Keep transcript links or exported transcripts private if they include proprietary code. The proof artifact should contain only paths, counts, commands, metadata, and aggregate metrics unless the repo is public and sharing transcript text is allowed.
