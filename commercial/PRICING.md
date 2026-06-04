# CallSieve Pricing

The CallSieve core local CLI, MCP server, indexer, retrieval, agent integrations, benchmark harness, and proof reports stay MIT-licensed and free. The OSS core continues to grow; nothing is being moved behind a paywall.

The three paid products below are proposed commercial packages. They are not current public CLI commands or shipped binaries unless a section explicitly says so. They define the pricing conversation around additive layers on top of the OSS core: cross-developer aggregation, security-grade audit streaming, and hands-on tuning for very large repositories.

All prices are placeholders during the current pricing-discovery phase. The numbers are real enough to transact on; expect them to settle, not swing.

---

## 1. Team Dashboard

A planned team dashboard would aggregate the `proof-report` JSON that individual developers already produce locally, then surface team-level retrieval quality and agent-cost trends. The intended V1 delivery is a self-hosted single binary backed by local SQLite. There would be no cloud dependency, no inbound network requirement, and no proprietary code leaving the machine running the binary. A hosted option is a later phase after self-hosted demand is proven.

| Field | Detail |
| --- | --- |
| Buyer | Engineering leaders who need to prove whether their agent investment is working. |
| What you get | Planned: per-team `first_correct_file_rate`, `turns_to_first_edit`, retrieval-quality drift alerts, token spend deltas, unlimited repos, support included. |
| Delivery | Planned: self-hosted single binary, local SQLite, no cloud dependency. Hosted option later. |
| How to start | Use `proof-report` and `evidence-pack` today; dashboard trials start after the self-hosted binary exists. |
| Price | **$20 per developer per month**, billed annually. |

---

## 2. Audit Log

A planned audit-log product would emit every agent action, every file the agent read, and every file CallSieve would have surfaced as a JSONL stream that Splunk, Datadog, and Sumo Logic could ingest through small per-platform connector binaries. It answers the security team's standing question about AI coding agents: "what did the agent touch, and when?" Repos with five or fewer active developers in the trailing 30 days (auto-detected from `git log`) would stay on a free tier so small projects are not penalized.

| Field | Detail |
| --- | --- |
| Buyer | Security teams at SOC2, FedRAMP, and regulated-industry orgs that must answer what the agent touched. |
| What you get | Planned: JSONL audit stream of every agent action, file read, and CallSieve-surfaced file, plus per-platform SIEM connector binaries for Splunk, Datadog, and Sumo. |
| Delivery | Planned: local-first stream ingested by the SIEM the security team already runs. No proprietary code leaves the host. |
| How to start | Use existing trace, `policy-check`, `proof-report`, and `evidence-pack` JSON today; SIEM connectors are planned. |
| Price | **$500 per repo per year**. Free tier for repos with five or fewer active developers in the trailing 30 days, auto-detected from `git log`. |

---

## 3. Monorepo Retrieval Tuning

A hands-on engagement for repos larger than 100,000 files, where the default ranker needs tuning, custom signal ingestion, and benchmark validation against the actual codebase. Output is a tuned `.callsieve/config.toml`, custom indexer signals where appropriate, a private benchmark suite covering the repo's task shapes, and a retrieval regression CI step. Patterns that recur across engagements get productized into the OSS core over time.

| Field | Detail |
| --- | --- |
| Buyer | Large engineering orgs with a single very large repo (FAANG-shaped, big-bank-shaped, big-game-studio-shaped). |
| What you get | Tuned ranker weights, custom signal ingestion, private benchmark suite, retrieval regression CI, written handoff. |
| Delivery | Remote engagement against your repo on your hardware. No code leaves your environment. |
| How to start | Book a call. This is the only product where that is the right answer. |
| Price | **$25,000 fixed-fee engagement**. Productized over time as repeatable patterns emerge. |

---

## FAQ

**Do I lose anything by using only the OSS core?**
No. The OSS core stays MIT and stays complete. The paid products are additive layers (team-level aggregation, SIEM-grade audit streams, large-monorepo tuning) that do not exist in the core because they are not the core's job.

**Do you take my code?**
No. The same local-first guarantees that apply to the OSS core apply to the proposed paid products: no cloud services, no API keys, no proprietary code leaves your machine. The planned team dashboard is self-hosted, the planned audit log streams to the SIEM you already run, and monorepo engagements run against your repo on your hardware.

**Can I migrate from self-hosted to cloud later?**
Yes, if a hosted option ships later. The planned self-hosted dashboard's SQLite store would be the source of truth, and the migration path would be an export-and-import against the same schema. Self-hosted would continue to be supported alongside any hosted option.
