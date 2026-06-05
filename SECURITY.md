# Security Policy

CallSieve is local-first software. It should not send proprietary code, traces, or repository data to a cloud service. Security reports should preserve that same standard.

## Supported Versions

Security fixes target the default branch and the latest published release when a release is affected. Older versions may receive fixes when the impact and maintenance cost justify it.

## Reporting a Vulnerability

Do not open a public issue with exploit details, secrets, private source code, proprietary traces, or customer data.

Use GitHub private vulnerability reporting or a private maintainer contact path when available. If private reporting is not available, open a minimal public issue that says you need a private security channel, without including sensitive details.

Please include:

- Affected version or commit.
- Operating system and environment details.
- Minimal reproduction steps.
- Expected and actual impact.
- Whether the issue can expose or transmit repository code, traces, tokens, credentials, or local files.
- Any suggested mitigation.

## Scope

Security-sensitive areas include:

- Local file discovery, indexing, and ignore handling.
- MCP server behavior and agent setup commands.
- Hooks, shims, and enforcement flows.
- Trace, pilot, proof-report, evidence-pack, and enterprise-proof-report handling.
- Any behavior that could expose proprietary source code, secrets, private traces, or local filesystem data.

## Response Expectations

Maintainers will acknowledge reports as promptly as practical, assess impact, and coordinate a fix or mitigation. Public disclosure should wait until maintainers have had a reasonable opportunity to investigate and release a fix.

## Safe Research

Use test repositories or redacted fixtures when possible. Do not test against systems, repositories, or data you do not own or have permission to evaluate.
