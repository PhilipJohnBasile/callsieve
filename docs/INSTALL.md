# Human Install And Integration Guide

This guide is for humans installing CallSieve and adding it to AI coding tools.

CallSieve is local-first. It indexes code on your machine, runs as a CLI or stdio MCP server, and does not require API keys or cloud services.

## Prerequisites

Required:

- Git
- Rust stable toolchain with Cargo

Recommended:

- `rust-analyzer` for Rust repos
- `typescript-language-server` for TypeScript and JavaScript repos
- `pyright-langserver` for Python repos

Language servers are optional. CallSieve falls back to tree-sitter and deterministic heuristics when they are missing.

## Install From GitHub Releases

Download the archive for your OS and CPU from:

```text
https://github.com/PhilipJohnBasile/callsieve/releases
```

Each release asset includes the `callsieve` binary plus this install guide, the AI CLI runbook, and a `.sha256` checksum file.

Recommended first check after unpacking:

```bash
callsieve --help
callsieve demo /path/to/repo --task "find where login sessions are created"
```

`demo` builds the local index, returns the first files an agent should read, and reports platform-neutral `context_payload_reduction` so you can verify the core loop before configuring an AI tool.

## Install From Source

Clone and install:

```bash
git clone https://github.com/PhilipJohnBasile/callsieve.git
cd callsieve
cargo install --path . --force
callsieve --help
```

If `callsieve` is not found after install, make sure Cargo's bin directory is on your PATH.

Windows:

```text
%USERPROFILE%\.cargo\bin
```

macOS and Linux:

```text
$HOME/.cargo/bin
```

Update later with:

```bash
cd /path/to/callsieve
git pull
cargo install --path . --force
```

Use without installing while developing CallSieve itself:

```bash
cargo run -- --help
cargo run -- index . --lsp
cargo run -- agent-context . "find the code that handles login"
cargo run -- demo . --task "find the code that handles login"
```

## First Repo Setup

From any repo you want an AI agent to work in:

```bash
callsieve index /path/to/repo --lsp
callsieve status /path/to/repo
callsieve agent-context /path/to/repo "find the code that handles login"
callsieve demo /path/to/repo --task "find the code that handles login"
```

If the output includes `read_first`, CallSieve is ready for that repo.

For one-command local adoption:

```bash
callsieve bootstrap /path/to/repo --client generic --strict --force --lsp
callsieve doctor /path/to/repo --client generic --strict
```

`bootstrap` writes local files under the repo only. It does not mutate global shell profiles, global PATH, cloud config, or user-wide app settings.

## Add To AI Tools

CallSieve supports two integration styles:

- MCP: the AI tool calls `callsieve_context`, `callsieve_symbol`, and related tools.
- CLI policy: the AI tool is instructed to run `callsieve agent-context <repo> "<task>"` before broad search.

Prefer MCP when the tool supports local stdio MCP. Use CLI policy everywhere else.

## Codex

Project setup:

```bash
callsieve agent-setup /path/to/repo --client codex --force
```

Codex-first setup with launchers and local shims:

```bash
callsieve codex-bootstrap /path/to/repo --model gpt-5-codex --force
callsieve doctor /path/to/repo --client codex --strict
```

Generated files include:

- `.codex/config.toml`
- `.codex/CALLSIEVE.md`
- `.callsieve/codex-launch.ps1`
- `.callsieve/codex-launch.sh`
- `.callsieve/bin/*` local wrappers

The project `.codex/config.toml` points Codex at:

```bash
callsieve mcp
```

The Codex policy tells the agent to use `callsieve_context` before broad grep or repeated file reads.

## Claude Code

Generate project files:

```bash
callsieve agent-setup /path/to/repo --client claude --force
```

Generated files:

- `.mcp.json`
- `CLAUDE.md`

Manual MCP equivalent:

```bash
claude mcp add --transport stdio callsieve -- callsieve mcp
```

Claude should call `callsieve_context` first for codebase discovery tasks.

## Claude Desktop

Claude Desktop MCP packaging changes more often than repo-local CLI usage. Use Claude Code for direct local stdio MCP today, or package CallSieve as a Desktop extension that launches:

```bash
callsieve mcp
```

The same tool rule applies: call `callsieve_context` before broad repo search.

## Cursor

Generate project files:

```bash
callsieve agent-setup /path/to/repo --client cursor --force
```

Generated files:

- `.cursor/mcp.json`
- `.cursor/rules/callsieve.mdc`

Cursor should use the project MCP config and call `callsieve_context` before repository-wide search.

You can also generate editor helper files:

```bash
callsieve editor-hook /path/to/repo --editor cursor --force
```

## Cline

Generate project files:

```bash
callsieve agent-setup /path/to/repo --client cline --force
```

Generated files:

- `.cline/mcp.json`
- `.clinerules/callsieve.md`

Cline should use `callsieve_context` before search tools and only grep when the packet is insufficient.

## Roo

Generate project files:

```bash
callsieve agent-setup /path/to/repo --client roo --force
```

Generated files:

- `.roo/mcp.json`
- `.roo/rules/callsieve.md`

Roo should use `callsieve_context` before broad search tools and repeated file reads.

## Gemini CLI, Kimi CLI, And Other AI CLIs

If the tool supports stdio MCP, ask CallSieve for a portable config:

```bash
callsieve mcp-config /path/to/repo --format json
callsieve mcp-config /path/to/repo --format toml
```

Use the format your AI CLI accepts. The JSON shape is:

```json
{
  "mcpServers": {
    "callsieve": {
      "type": "stdio",
      "command": "callsieve",
      "args": ["mcp"]
    }
  }
}
```

`agent-setup --client generic` writes the same reusable config files under the repo:

```bash
callsieve agent-setup /path/to/repo --client generic --force
```

Generated files:

- `.callsieve/mcp.json`
- `.callsieve/mcp.toml`
- `.callsieve/agent-policy.md`

If the tool does not support MCP, add this policy to its project instructions:

```text
Before broad search or repeated file reads, run:
callsieve agent-context <repo> "<task>" --limit 8 --snippets-per-file 2

Read the returned read_first files and snippets first.
Use grep only when that packet is insufficient.
When reporting savings, call context_payload_reduction an estimated context payload reduction, not observed session token savings.
```

## Strict Grep Shims

For stronger local enforcement, install repo-local grep wrappers:

```bash
callsieve shim install /path/to/repo --force --strict
callsieve shim doctor /path/to/repo
```

Then prepend this directory to the PATH used by the AI tool process only:

```text
/path/to/repo/.callsieve/bin
```

Do not add this globally unless you intentionally want CallSieve shims for every shell. The safer pattern is process-local PATH through a launcher.

Remove shims with:

```bash
callsieve shim uninstall /path/to/repo
```

## Verify An Integration

Run:

```bash
callsieve doctor /path/to/repo --client <codex|claude|cursor|cline|roo|generic> --strict
callsieve enforce /path/to/repo --client <codex|claude|cursor|cline|roo|generic> --strict
```

Expected healthy signals:

- index exists and is fresh
- generated agent policy/config files exist
- MCP command points at CallSieve
- strict shim state is present when required
- trace policy passes when a trace is supplied

Smoke test the agent-facing command:

```bash
callsieve demo /path/to/repo --task "find where login sessions are created"
callsieve agent-context /path/to/repo "find where login sessions are created" --limit 8 --snippets-per-file 2
callsieve mcp-config /path/to/repo --format json
```

Smoke test MCP:

```bash
callsieve mcp
```

The MCP command runs a stdio server. It waits for a client and does not print a normal interactive prompt.

## Daily Human Workflow

For a repo you actively work in:

```bash
callsieve index /path/to/repo --lsp
callsieve daemon /path/to/repo --background --lsp
callsieve daemon-status /path/to/repo
```

Before handing a task to an AI:

```bash
callsieve agent-context /path/to/repo "<task>"
```

`agent-context` keeps a small local `.callsieve/task-memory.json` hint cache so repeated task families can reuse prior read-first files and symbols. Clear it when you want a cold run:

```bash
callsieve memory-clear /path/to/repo
```

After a task, audit whether the session followed CallSieve-first policy if you have a trace:

```bash
callsieve trace-check /path/to/trace.json --strict
```

## Evidence Setup

Run the deterministic rehearsal:

```bash
callsieve proof-rehearsal --preflight
callsieve proof-rehearsal --fix --resume
```

Set up the observed Codex milestone:

```bash
callsieve setup-observed-codex-oss-50
callsieve pilot-qa benchmarks/evidence/observed-codex-oss-50.local.json
```

`pilot-qa` should fail until real observed paired sessions are recorded. Do not run `proof-report` as claim proof until `pilot-qa` passes.

## Troubleshooting

`callsieve` is not found:

- Confirm Cargo bin is on PATH.
- Use the absolute path to the installed binary in MCP config.
- In this repo, use `cargo run -- <command>` as a fallback.

Index is missing:

```bash
callsieve index /path/to/repo --lsp
```

MCP client cannot start CallSieve:

- Replace `"command": "callsieve"` with the absolute binary path.
- Run `callsieve agent-setup /path/to/repo --client <client> --force` to regenerate config with the resolved executable path.

Windows tests fail with `Access is denied` for `callsieve.exe`:

```bash
callsieve daemon-stop /path/to/repo
```

If a stale AI tool process still holds the binary, close that process and rerun the test.
