# Enterprise Proof Protocol

This protocol is for proving the broad claim that CallSieve materially reduces developer-session grep/read token waste across real agent workflows.

Do not use "almost all developer sessions" or broad enterprise language until the enterprise-proof report status is `pass`.

## Evidence Tiers

- `local_harness`: controlled local proof and shakedown evidence. Useful, but not enough for broad claims.
- `oss_scale_proxy`: large public repositories used as Microsoft-scale proxies.
- `private_enterprise`: private enterprise repositories, when teams can share audited aggregate evidence.
- `paid_pilot`: real teams using CallSieve repeatedly in pilot workflows.

Private enterprise evidence is additive. The initial scale proof can pass with large OSS proxies when private repositories are not available.

## Hard Gates

The broad claim requires all of these gates:

- At least `1,000` paired observed sessions.
- At least `50` repositories.
- At least `10` Microsoft-scale OSS proxy repositories.
- At least `3` agent clients, with `codex`, `claude`, and `cursor` present.
- At least `5` languages or ecosystems.
- At least `10` task categories.
- Aggregate observed token reduction at least `50%`.
- At least `90%` of observed sessions have positive token savings.
- At least `75%` of observed sessions have more than `30%` token savings.
- Critical misses: `0`.
- Strict trace-policy violations: `0`.
- Controlled replay ratio: `0`.
- Transcript-token-accounted observed sessions: `100%`.

`enterprise-proof-report` enforces these gates with report fields under `proof`: `observed_sessions`, `scale_proxy_repos`, `clients`, `languages`, `task_categories`, `positive_savings_session_percent`, `sessions_over_30_percent_savings_percent`, `controlled_replay_ratio`, `critical_files_still_missed`, `trace_policy_violations`, `transcript_token_accounting_percent`, `per_client`, `per_scale_class`, and `per_task_category`.

Product-market gates:

- At least `5` real teams complete pilots.
- At least `3` teams are paid pilots or convert to paid.
- At least `4` teams use CallSieve in `20+` real sessions.
- At least `3` teams report they would be meaningfully worse off without it.
- At least `2` teams provide quote-approved case studies or anonymized evidence packs.
- At least `1` team renews, expands, or signs a letter of intent.

## Microsoft-Scale Proxy Criteria

A scale proxy repository must have at least one of:

- `1M+` LOC.
- `100k+` files.
- `1k+` modules, packages, or crates.
- Multi-language monorepo structure.
- Build or test graph complexity comparable to enterprise repositories.

Use public proxies such as Chromium-scale, Kubernetes-scale, VS Code-scale, TypeScript-scale, Rust-scale, and similar large OSS repositories that can be cloned and indexed locally. Do not require Microsoft-owned repositories for the initial proof.

## Collection Protocol

For each task pair:

1. Run the baseline phase first with the normal agent workflow and no CallSieve read-first context.
2. Run the CallSieve phase second with `callsieve agent-context <repo> "<task>"` before broad grep or repeated reads.
3. Record client, model, exact files read, commands, token counts, task category, repo scale class, and transcript token-accounting source.
4. Reject contaminated sessions with an audit reason instead of deleting them.
5. Run `callsieve pilot-qa <manifest>` every `25` paired sessions.

Only observed sessions with `metadata.collection = "observed_session"` count toward the broad proof. Controlled replay is reported separately and must be zero for the enterprise gate.

Each repo entry should declare `proof_tier`, `scale_class`, `scale_criteria`, `languages`, `clients`, and `task_categories`. The report also derives client and task-category coverage from observed traces and suite metadata, but manifest metadata makes gaps visible before collection is complete.

## Scale Validation

For each scale proxy:

1. Run `callsieve index <repo> --lsp` when local language servers are available.
2. Run `callsieve eval-retrieval <manifest.json>` for recall and critical-file miss checks.
3. Run `callsieve perf-report <repo> --tasks <manifest.json>` for local p50/p95 context latency.
4. Measure index time, index size, freshness, query latency, context packet size, recall, and failure rate.
5. Record `audit.scale_validation.agent_context_p95_latency_ms`.
6. Keep index failures, stale-index failures, and crashes at `0`.

The default p95 `agent-context` latency gate is `5000ms` on developer hardware.

## Product-Market Validation

For each pilot team:

1. Package `callsieve evidence-pack <manifest> --anonymize`.
2. Track activation, repeat usage, retained usage, paid conversion, renewal or LOI, and "meaningfully worse without it" response.
3. Keep commercial proof separate from technical proof, then combine the aggregate PMF metrics in the final enterprise manifest.

`evidence-pack --anonymize` redacts repo paths, labels, teams, and case-study identifiers while preserving aggregate proof and PMF metrics.

## Commands

Start from the example manifest:

```bash
cargo run -- enterprise-proof-report benchmarks/evidence/enterprise-proof-manifest.example.json
```

For shareable pilot evidence:

```bash
cargo run -- evidence-pack benchmarks/evidence/enterprise-proof-manifest.example.json --anonymize
```

The report status must be `pass` before using the broad claim.

For normal local pilot claims, keep using `proof-report`. Use `enterprise-proof-report` only when the manifest is intentionally collecting broad developer-session, scale, and PMF evidence.
