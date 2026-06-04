# Human Install And Integration Guide

This guide is for humans installing CallSieve and adding it to AI coding tools.

CallSieve is local-first. It indexes code on your machine, runs as a CLI, stdio MCP server, or repo-local hook layer, and does not require API keys or cloud services. Retrieval spends zero AI model tokens because ranking runs against the local index; returned context still counts when an agent reads it.

## Prerequisites

Required:

- Git
- Rust stable toolchain with Cargo

Recommended:

- `rust-analyzer` for Rust repos
- `typescript-language-server` for TypeScript and JavaScript repos
- `pyright-langserver` for Python repos
- `intelephense` for PHP repos
- `gopls` for Go repos
- `clangd` for C and C++ repos
- `ruby-lsp` for Ruby repos
- `lua-language-server` for Lua repos
- `csharp-ls`, `jdtls`, `kotlin-language-server`, `sourcekit-lsp`, `metals`, or `dart` for C#, Java, Kotlin, Swift, Scala, or Dart repos

Language servers are optional. CallSieve falls back to tree-sitter and deterministic heuristics when they are missing.

## Install From GitHub Releases

Download the archive for your OS and CPU from:

```text
https://github.com/PhilipJohnBasile/callsieve/releases
```

Each release asset includes the `callsieve` binary plus the changelog, this install guide, the AI CLI runbook, and a `.sha256` checksum file.

Recommended first check after unpacking:

```bash
callsieve --help
callsieve demo /path/to/repo --task "find where login sessions are created"
```

`demo` builds the local index, returns the first files an agent should read, exposes `retrieval_cost.retrieval_model_tokens = 0`, and reports platform-neutral `context_payload_reduction` so you can verify the core loop before configuring an AI tool.

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

For the hook-first setup testers usually want, install repo-local launchers:

```bash
callsieve hook install /path/to/repo --client generic --strict --force --lsp
callsieve hook doctor /path/to/repo
```

This writes `.callsieve/agent-launch.ps1`, `.callsieve/agent-launch.sh`, local shims, policy files, and MCP config. The launchers start the daemon, prepend `.callsieve/bin` only for the launched process, and then run the agent command passed to them.

## Add To AI Tools

CallSieve supports four integration styles:

- Lifecycle hooks and plugins: Codex, Claude Code, GitHub Copilot, OpenCode, Antigravity CLI, and Cline can inject CallSieve context and block pre-context broad search.
- MCP: the AI tool calls `callsieve_context`, `callsieve_symbol`, and related tools.
- CLI policy: the AI tool is instructed to run `callsieve agent-context <repo> "<task>"` before broad search.
- Hook launcher: the AI tool is started through `.callsieve/agent-launch.ps1` or `.callsieve/agent-launch.sh`, so repo-local shims can intercept broad `rg` and `grep`.

Prefer lifecycle hooks or plugins when the client supports them. Prefer hook launchers when you control how another agent process starts. Prefer MCP when the tool supports local stdio MCP. Use CLI policy everywhere else.

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

Hook-first Codex setup:

```bash
callsieve codex-hooks install /path/to/repo --strict --force
callsieve codex-hooks doctor /path/to/repo --strict --smoke
callsieve codex-hooks trust-ack /path/to/repo
```

Generated files include:

- `.codex/config.toml`
- `.codex/CALLSIEVE.md`
- `.codex/hooks.json`
- `.callsieve/codex-launch.ps1`
- `.callsieve/codex-launch.sh`
- `.callsieve/bin/*` local wrappers

The project `.codex/config.toml` points Codex at:

```bash
callsieve mcp
```

The Codex policy tells the agent to use `callsieve_context` before broad grep or repeated file reads. Codex hooks use the `slim` profile. The project `.codex/hooks.json` runs local `callsieve codex-hook ...` handlers. `UserPromptSubmit` injects compact CallSieve context, `PreToolUse` blocks broad search before context, and `PermissionRequest` denies escalated pre-context search. Codex `PostToolUse` and `Stop` are intentionally not installed because pre-tool hooks enforce the policy and post-tool or stop-time prompts are optional. Run `codex-hooks doctor --strict --smoke` to validate the local handler contract. Add `--fix` to archive stale hook state or trace files under `.callsieve/codex-hooks/archive/`. Review and trust project hooks in Codex with `/hooks`, then run `codex-hooks trust-ack /path/to/repo` to record a local marker tied to the current hook file hash.

## Claude Code

Hook, shim, and MCP setup:

```bash
callsieve hook install /path/to/repo --client claude --strict --force --lsp
callsieve claude-hooks doctor /path/to/repo --strict
callsieve enforce /path/to/repo --client claude --strict
```

Generated files:

- `.mcp.json`
- `CLAUDE.md`
- `.claude/settings.local.json`
- `.callsieve/agent-launch.ps1`
- `.callsieve/agent-launch.sh`
- `.callsieve/bin/*` local wrappers

Hook-only setup:

```bash
callsieve claude-hooks install /path/to/repo --strict --force
callsieve claude-hooks doctor /path/to/repo --strict
```

Manual MCP equivalent:

```bash
claude mcp add --transport stdio callsieve -- callsieve mcp
```

Claude should call `callsieve_context` first for codebase discovery tasks. The generated `.claude/settings.local.json` preserves unrelated local settings and adds `callsieve claude-hook ...` handlers. `UserPromptSubmit` injects compact CallSieve context, `PreToolUse` blocks `Bash`, `Read`, `Grep`, and `Glob` before context in strict mode, `PostToolUse` records guardrail trace events, `PermissionRequest` denies escalated pre-context search, and `Stop` stays quiet with a suppressed acknowledgement. Review and trust project hooks in Claude Code with `/hooks` before relying on enforcement.

## Claude Desktop

Claude Desktop MCP packaging changes more often than repo-local CLI usage. Use Claude Code for direct local stdio MCP today, or package CallSieve as a Desktop extension that launches:

```bash
callsieve mcp
```

The same tool rule applies: call `callsieve_context` before broad repo search.

## GitHub Copilot

Local Copilot CLI setup:

```bash
callsieve hook install /path/to/repo --client copilot --strict --force --lsp
callsieve copilot-hooks doctor /path/to/repo --strict
callsieve enforce /path/to/repo --client copilot --strict
```

Generated files:

- `.github/copilot-mcp.json`
- `.github/copilot-instructions.md`
- `.github/agents/callsieve-context.agent.md`
- `.github/hooks/callsieve.json`
- `.callsieve/agent-launch.*`
- `.callsieve/bin/*`

The hook file runs `callsieve copilot-hook ...` handlers for prompt context, pre-tool blocking, post-tool tracing, permission decisions, and stop/session events. Copilot cloud agents are template-only unless the local CallSieve binary is available inside that sandbox.

## OpenCode

Hook, plugin, and MCP setup:

```bash
callsieve hook install /path/to/repo --client opencode --strict --force --lsp
callsieve opencode-hooks doctor /path/to/repo --strict
callsieve enforce /path/to/repo --client opencode --strict
```

Generated files:

- `opencode.json`
- `.opencode/CALLSIEVE.md`
- `.opencode/plugins/callsieve.js`
- `.callsieve/agent-launch.*`
- `.callsieve/bin/*`

`opencode.json` preserves unrelated settings and upserts `mcp.callsieve` plus the CallSieve instruction file. The plugin uses `tool.execute.before`, `tool.execute.after`, and session events to call local `callsieve opencode-hook ...` handlers.

## Antigravity CLI

Hook, MCP, skill, and rule setup:

```bash
callsieve hook install /path/to/repo --client antigravity --strict --force --lsp
callsieve antigravity-hooks doctor /path/to/repo --strict
callsieve enforce /path/to/repo --client antigravity --strict
```

Generated files:

- `.agents/mcp_config.json`
- `.agents/hooks.json`
- `.agents/skills/callsieve-context.md`
- `.agents/rules/callsieve.md`
- `.callsieve/agent-launch.*`
- `.callsieve/bin/*`

The generated hooks use `PreInvocation`, `PreToolUse`, `PostToolUse`, and `Stop` events and call local `callsieve antigravity-hook ...` handlers. Keep `GEMINI.md` or `AGENTS.md` compatibility docs as migration notes rather than treating Gemini CLI as a separate first-class target.

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

Hook, shim, MCP, and rules setup:

```bash
callsieve hook install /path/to/repo --client cline --strict --force --lsp
callsieve cline-hooks doctor /path/to/repo --strict
callsieve enforce /path/to/repo --client cline --strict
```

Generated files:

- `.cline/mcp.json`
- `.cline/rules/callsieve.md`
- `.clinerules/callsieve.md`
- `.cline/hooks/*`
- `.callsieve/agent-launch.*`
- `.callsieve/bin/*`

Cline should use `callsieve_context` before search tools and only grep when the packet is insufficient. The generated hook scripts call local `callsieve cline-hook ...` handlers for prompt context, pre-tool blocking, post-tool tracing, permission decisions, and stop/task completion.

## Zoo Code

Generate project files:

```bash
callsieve agent-setup /path/to/repo --client zoo --force
```

Generated files:

- `.roo/mcp.json`
- `.roo/rules/callsieve.md`
- `.roo/rules-code/callsieve.md`
- `.roomodes` only when the repo already has one or `--force` is used

Zoo currently uses the `.roo` config paths documented by Zoo Code. Strict mode requires MCP, rules, a fresh index, and local shims, but no lifecycle hooks are required. `--client roo` remains a deprecated alias that generates the same Zoo-compatible `.roo/*` files and emits a warning.

## VS Code, Windsurf, Continue, Zed, Junie, JetBrains, Amp, Goose, and Warp

These clients are MCP/rules/skills/setup-template targets only:

```bash
callsieve agent-setup /path/to/repo --client vscode --force
callsieve agent-setup /path/to/repo --client windsurf --force
callsieve agent-setup /path/to/repo --client continue --force
callsieve agent-setup /path/to/repo --client zed --force
callsieve agent-setup /path/to/repo --client junie --force
callsieve agent-setup /path/to/repo --client jetbrains --force
callsieve agent-setup /path/to/repo --client amp --force
callsieve agent-setup /path/to/repo --client goose --force
callsieve agent-setup /path/to/repo --client warp --force
```

Generated files are project-local. VS Code, Junie, and valid Zed JSON settings preserve unrelated fields when regenerated with `--force`. If Zed settings are JSONC or invalid JSON, CallSieve leaves `.zed/settings.json` untouched and writes `.callsieve/integrations/zed-settings.json` as a template with a warning. JetBrains AI Assistant setup is docs/template-only; use `--client junie` for Junie. Warp cloud-agent setup is template-only unless the Warp/Oz runtime can run the local `callsieve` binary.

## Other Stdio MCP AI CLIs

If the tool supports stdio MCP, ask CallSieve for a portable config:

```bash
callsieve mcp-config /path/to/repo --format json
callsieve mcp-config /path/to/repo --format toml
callsieve mcp-registry-manifest --out server.json
```

Use the format your AI CLI accepts. `mcp-registry-manifest` writes a local-first MCP Registry `server.json` descriptor for `callsieve mcp`; it does not contact the network or publish automatically. The JSON config shape is:

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
callsieve agent-context <repo> "<task>"

Read the returned read_first files and snippets first.
Use grep only when that packet is insufficient.
Treat retrieval_cost.retrieval_model_tokens = 0 as retrieval-only; returned context still counts when read.
When reporting savings, call context_payload_reduction an estimated context payload reduction, not observed session token savings.
```

## Strict Grep Shims

For stronger local enforcement, use the hook installer:

```bash
callsieve hook install /path/to/repo --client generic --strict --force --lsp
callsieve hook doctor /path/to/repo
```

Run an agent through the generated launcher:

```bash
/path/to/repo/.callsieve/agent-launch.sh <agent-command> <args>
```

On Windows PowerShell:

```powershell
& C:\path\to\repo\.callsieve\agent-launch.ps1 <agent-command> <args>
```

If you only need the low-level wrappers, install repo-local grep shims directly:

```bash
callsieve shim install /path/to/repo --force --strict
callsieve shim doctor /path/to/repo
```

Then prepend this directory to the PATH used by the AI tool process only:

```text
/path/to/repo/.callsieve/bin
```

Do not add this globally unless you intentionally want CallSieve shims for every shell. The safer pattern is process-local PATH through a launcher.

The wrappers call the hidden `callsieve shim-run` helper. It extracts the search pattern from common `rg` and `grep` forms, returns CallSieve context first, and then passes the original arguments through to the real search binary captured at install time.

Remove shims with:

```bash
callsieve shim uninstall /path/to/repo
```

## Verify An Integration

Run:

```bash
callsieve doctor /path/to/repo --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> --strict
callsieve enforce /path/to/repo --client <codex|claude|copilot|opencode|antigravity|cursor|vscode|windsurf|continue|zed|junie|jetbrains|amp|goose|warp|cline|zoo|roo|generic> --strict
```

Expected healthy signals:

- index exists and is fresh
- generated agent policy/config files exist
- lifecycle hooks or plugins exist for Codex, Claude Code, Copilot, OpenCode, Antigravity, and Cline in strict mode
- Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, and Zoo strict mode do not require lifecycle hooks
- MCP command points at CallSieve
- strict shim state is present when required
- trace policy passes when a trace is supplied

Smoke test the agent-facing command:

```bash
callsieve demo /path/to/repo --task "find where login sessions are created"
callsieve agent-context /path/to/repo "find where login sessions are created"
callsieve mcp-config /path/to/repo --format json
callsieve mcp-registry-manifest --out server.json
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
