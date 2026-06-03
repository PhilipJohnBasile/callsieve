# MCP Setup

`callsieve mcp` runs a local stdio MCP server. It exposes:

- `callsieve_context`
- `callsieve_symbol`
- `callsieve_stats`
- `callsieve_status`
- `callsieve_trace_check`
- `callsieve_benchmark`

For human installation and client setup, start with [INSTALL.md](INSTALL.md). For AI CLI behavior and automation rules, see [AGENT_CLI.md](AGENT_CLI.md).

The MCP server is one integration surface for agents. It does not replace the CLI: indexing, lifecycle hooks or plugins, hook launchers, Markdown or JSON context output, watching, daemon refresh, evidence collection, proof reports, and enterprise-proof reports still run through `callsieve` commands.

Build or install CallSieve first. You can index each repository up front, or let the first `callsieve_context` call rebuild a missing or stale local index:

```bash
cargo install --path .
callsieve index /path/to/repo
callsieve mcp
```

For higher-confidence reference edges, index with local LSP enrichment before starting MCP:

```bash
callsieve index /path/to/repo --lsp
```

`callsieve_context` checks freshness before ranking. If `.callsieve/index.json` is missing or stale, it rebuilds and saves the local index, then returns the context packet. The response includes `freshness.initial_fresh`, `freshness.refreshed`, `freshness.final_fresh`, `freshness.index_generation`, `freshness.stale_files`, and `freshness.fix_command`, plus timing fields such as `freshness_check_ms`, `index_rebuild_ms`, and `mcp_total_ms`.

The MCP server does not install language servers, install grep shims, mutate client config, mutate traces, start the daemon, or send code to a remote service. If an MCP rebuild fails, the tool response returns the exact CLI repair command in `structuredContent.error.fix_command`, for example:

```bash
callsieve index /path/to/repo
```

If you do not install the binary, replace `callsieve` in the examples with:

```bash
cargo run --manifest-path /path/to/callsieve/Cargo.toml -- mcp
```

CallSieve can generate local config and a before-grep policy file for supported clients:

```bash
callsieve bootstrap /path/to/repo --client generic --strict --force
callsieve doctor /path/to/repo --client generic --strict
callsieve doctor /path/to/repo --client generic --fix --strict
callsieve agent-setup /path/to/repo --client codex
callsieve agent-setup /path/to/repo --client claude
callsieve agent-setup /path/to/repo --client copilot
callsieve agent-setup /path/to/repo --client opencode
callsieve agent-setup /path/to/repo --client antigravity
callsieve agent-setup /path/to/repo --client cursor
callsieve agent-setup /path/to/repo --client vscode
callsieve agent-setup /path/to/repo --client windsurf
callsieve agent-setup /path/to/repo --client continue
callsieve agent-setup /path/to/repo --client zed
callsieve agent-setup /path/to/repo --client junie
callsieve agent-setup /path/to/repo --client jetbrains
callsieve agent-setup /path/to/repo --client amp
callsieve agent-setup /path/to/repo --client goose
callsieve agent-setup /path/to/repo --client warp
callsieve agent-setup /path/to/repo --client cline
callsieve agent-setup /path/to/repo --client zoo
callsieve agent-setup /path/to/repo --client roo
callsieve agent-setup /path/to/repo --client generic
```

Pass `--force` to replace existing generated files.
Generated MCP configs use the resolved CallSieve executable path so client startup does not depend on the agent shell PATH. Generated policy files also include the first command agents should run for every task: `callsieve agent-context <repo> "<task>"`. Manual examples below use `callsieve` for readability; replace it with an absolute path when the client shell cannot resolve the binary.

For AI CLIs without a dedicated setup command, print a portable MCP config:

```bash
callsieve mcp-config /path/to/repo --format json
callsieve mcp-config /path/to/repo --format toml
callsieve mcp-registry-manifest --out server.json
```

Audit generated setup with:

```bash
callsieve enforce /path/to/repo --client codex
callsieve enforce /path/to/repo --client codex --trace /path/to/trace.json --strict
```

`enforce` checks index freshness, generated client policy/config files, optional trace policy, lifecycle hook surfaces where supported, and shim state. Strict mode requires hook/plugin files for Codex, Claude Code, Copilot, OpenCode, Antigravity, and Cline. Cursor, VS Code, Windsurf, Continue, Zed, Junie, JetBrains AI Assistant, Amp, Goose, Warp, and Zoo do not require lifecycle hooks. Generic clients can fail missing shims with `--require-shim`.

## Codex

Codex supports stdio MCP servers through `config.toml`.

Project-scoped `.codex/config.toml`:

```toml
[mcp_servers.callsieve]
command = "/absolute/path/to/callsieve"
args = ["mcp"]
startup_timeout_sec = 20
tool_timeout_sec = 60
```

CLI equivalent:

```bash
codex mcp add callsieve -- callsieve mcp
```

Use `callsieve_context` as the first codebase-discovery tool before broad grep or repeated file reads.

For Codex/ChatGPT-only evidence collection, start measured tasks with:

```bash
callsieve codex-session /path/to/repo "change login token expiry behavior" --trace-out /path/to/repo/.callsieve/codex-session.json --model gpt-5-codex
callsieve enforce /path/to/repo --client codex --trace /path/to/repo/.callsieve/codex-session.json --strict
```

Use a different `--model` label for each Codex/ChatGPT model you test. `codex-session` is controlled replay evidence: useful for setup checks, but not counted as observed-session proof. For claim-counted sessions, use `session-start`, `session-event`, and `session-finish` with transcript token accounting.

For enforcement in Codex, prefer lifecycle hooks over MCP alone:

```bash
callsieve codex-hooks install /path/to/repo --strict --force
callsieve codex-hooks doctor /path/to/repo --strict
```

The generated `.codex/hooks.json` injects context at prompt submit, blocks broad search before context, records local hook traces, and asks Codex to continue after strict violations. Review and trust project hooks in Codex with `/hooks`.

Reference: https://developers.openai.com/codex/mcp

## Claude Code

Claude Code can add a local stdio server with:

```bash
claude mcp add --transport stdio callsieve -- callsieve mcp
```

Project-scoped `.mcp.json`:

```json
{
  "mcpServers": {
    "callsieve": {
      "type": "stdio",
      "command": "callsieve",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

Reference: https://code.claude.com/docs/en/mcp

Ask Claude Code to call `callsieve_context` before `rg` for codebase discovery tasks, then read the returned `read_first` files.

For enforcement in Claude Code, prefer lifecycle hooks plus repo-local shims over MCP alone:

```bash
callsieve hook install /path/to/repo --client claude --strict --force --lsp
callsieve claude-hooks doctor /path/to/repo --strict
callsieve enforce /path/to/repo --client claude --strict
```

The generated `.claude/settings.local.json` preserves unrelated local Claude settings, injects context at prompt submit, blocks pre-context `Bash`, `Read`, `Grep`, and `Glob` in strict mode, and records local hook traces under `.callsieve/claude-hooks/`. The same install also writes `.mcp.json`, `CLAUDE.md`, `.callsieve/agent-launch.*`, and `.callsieve/bin/*`. Review and trust project hooks in Claude Code with `/hooks`.

Reference: https://code.claude.com/docs/en/hooks

## Claude Desktop

Claude Desktop's current local MCP path is desktop extensions. For broad Desktop distribution, package CallSieve as an `.mcpb` extension that launches `callsieve mcp`.

Until that package exists, use Claude Code or another stdio MCP client for direct local CallSieve access.

Reference: https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop

## GitHub Copilot

Generate local Copilot CLI files with:

```bash
callsieve hook install /path/to/repo --client copilot --strict --force --lsp
callsieve copilot-hooks doctor /path/to/repo --strict
```

This writes `.github/copilot-mcp.json`, `.github/copilot-instructions.md`, `.github/agents/callsieve-context.agent.md`, and `.github/hooks/callsieve.json`. Local hooks call `callsieve copilot-hook ...`. Copilot cloud agents remain template-only unless the local `callsieve` binary is available in that sandbox.

## OpenCode

Generate local OpenCode MCP and plugin files with:

```bash
callsieve hook install /path/to/repo --client opencode --strict --force --lsp
callsieve opencode-hooks doctor /path/to/repo --strict
```

This preserves unrelated `opencode.json` settings while adding `mcp.callsieve` and `.opencode/CALLSIEVE.md`. The plugin `.opencode/plugins/callsieve.js` uses tool and session events to call `callsieve opencode-hook ...`.

## Antigravity CLI

Generate Antigravity MCP, hooks, skill, and rule files with:

```bash
callsieve hook install /path/to/repo --client antigravity --strict --force --lsp
callsieve antigravity-hooks doctor /path/to/repo --strict
```

This writes `.agents/mcp_config.json`, `.agents/hooks.json`, `.agents/skills/callsieve-context.md`, and `.agents/rules/callsieve.md`. The hook config calls `callsieve antigravity-hook ...`.

## Cursor

Project-scoped `.cursor/mcp.json`:

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

Global config uses `~/.cursor/mcp.json`.

In Cursor, call `callsieve_context` before repository-wide search when starting a coding task.

Reference: https://docs.cursor.com/en/context/mcp

## VS Code, Windsurf, Continue, Zed, Junie, JetBrains, Amp, Goose, and Warp

These clients are MCP/rules/skills/setup-template targets only. Generate project-local setup with:

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

Generated files:

- VS Code: `.vscode/mcp.json` plus `.github/copilot-instructions.md`
- Windsurf: `.windsurf/rules/callsieve.md` plus `.callsieve/integrations/windsurf-mcp.json`
- Continue: `.continue/mcpServers/callsieve.yaml` plus `.continue/rules/callsieve.md`
- Zed: merged `.zed/settings.json` when it is valid JSON, otherwise `.callsieve/integrations/zed-settings.json` with a warning
- Junie: `.junie/mcp/mcp.json` plus `.junie/guidelines.md`
- JetBrains AI Assistant: `.callsieve/integrations/jetbrains-mcp.json` with a docs-only warning; use `--client junie` for Junie
- Amp: `.agents/skills/callsieve-context/SKILL.md` plus `.agents/skills/callsieve-context/mcp.json`
- Goose: `.callsieve/integrations/goose-config.yaml` plus `.callsieve/integrations/goose-deeplink.txt`
- Warp: `.callsieve/integrations/warp-mcp.json` plus `.callsieve/integrations/warp-agent.yaml`

Strict mode requires these generated files, a fresh index, daemon state, and shims. It does not require lifecycle hooks. Global/user config files are not mutated automatically. Warp cloud-agent templates are only usable when the Warp/Oz runtime can run the local `callsieve` binary.

## Cline

Cline supports local stdio servers under `mcpServers`.

Example `~/.cline/mcp.json`:

```json
{
  "mcpServers": {
    "callsieve": {
      "command": "callsieve",
      "args": ["mcp"],
      "env": {},
      "disabled": false,
      "autoApprove": []
    }
  }
}
```

Reference: https://docs.cline.bot/mcp/mcp-overview

In Cline, use `callsieve_context` before broad search tools and only grep when the returned packet is insufficient.

For strict Cline enforcement, generate hook scripts too:

```bash
callsieve hook install /path/to/repo --client cline --strict --force --lsp
callsieve cline-hooks doctor /path/to/repo --strict
```

## Zoo Code

Zoo Code uses generated `.roo` config paths:

```bash
callsieve agent-setup /path/to/repo --client zoo
```

Generated files include `.roo/mcp.json`, `.roo/rules/callsieve.md`, and `.roo/rules-code/callsieve.md`. Use `callsieve_context` before broad search tools and repeated file reads. The deprecated `--client roo` alias still generates the same Zoo-compatible files and emits a warning.

## Tool Workflow

For coding tasks, agents should:

1. Call `callsieve_context` with `{ "path": "/path/to/repo", "task": "..." }`, or run `callsieve begin /path/to/repo "<task>" --client <client> --trace-out /path/to/repo/.callsieve/session-trace.json` before any broad search.
2. Call `callsieve_status` if freshness or LSP enrichment state is uncertain.
3. Read the returned `read_first` snippets and files.
4. Use `callsieve_symbol` for named symbols when needed.
5. Grep only when the context packet is insufficient.

The `callsieve_context` tool metadata marks it as the preferred first tool for codebase discovery. Its practical instruction is: read these files first; grep only if insufficient.

Use `callsieve_trace_check` on captured trace JSON to detect sessions that ran grep before CallSieve. Pass `"strict": true` to also fail common file reads before `callsieve_context`.

For proof work, pair MCP usage with CLI trace collection. The agent should call `callsieve_context` first, then the operator should record the exact commands, files read, client, model, and token counts in observed-session traces.

## Hook Launchers

If you control how the agent process starts, install repo-local hooks:

```bash
callsieve hook install /path/to/repo --client generic --strict --force --lsp
callsieve hook doctor /path/to/repo
```

The install writes `.callsieve/agent-launch.ps1` and `.callsieve/agent-launch.sh`. Those launchers start the daemon, prepend `.callsieve/bin` only for that launched process, and then run the agent command passed to them. This gives MCP-capable and non-MCP agents the same process-local before-grep guardrails without mutating global PATH or shell profiles.

## Grep Shims

For opt-in PATH-level interception, install local wrappers:

```bash
callsieve shim install /path/to/repo --force --strict
callsieve shim doctor /path/to/repo
```

Then prepend `/path/to/repo/.callsieve/bin` to the PATH used by the agent shell for that process. The install writes a local `callsieve` launcher plus `rg` and `grep` wrappers. The search wrappers call the hidden `callsieve shim-run` helper, return CallSieve context for the extracted search pattern, and then pass the original arguments through to the real `rg` or `grep` command captured during install. With `--strict`, shim-mediated grep writes `.callsieve/shim-trace.json` for strict trace audits. Remove wrappers with:

```bash
callsieve shim uninstall /path/to/repo
```

## Daemon State

For a stateful local index refresh loop:

```bash
callsieve daemon /path/to/repo --foreground --lsp
callsieve daemon-status /path/to/repo
callsieve daemon-stop /path/to/repo
```

The daemon writes `.callsieve/daemon.json`, which `status`, `enforce`, and pilot workflows can use as operational evidence.

On Windows, a running daemon, MCP server, or shim-launched `callsieve.exe` can hold `target\debug\callsieve.exe` and make `cargo test` fail with `Access is denied`. Run `callsieve daemon-stop /path/to/repo` first. If a stale process still holds the binary, terminate `callsieve.exe` and rerun the test command.
