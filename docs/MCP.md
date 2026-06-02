# MCP Setup

`callsieve mcp` runs a local stdio MCP server. It exposes:

- `callsieve_context`
- `callsieve_symbol`
- `callsieve_stats`
- `callsieve_status`
- `callsieve_trace_check`
- `callsieve_benchmark`

For human installation and client setup, start with [INSTALL.md](INSTALL.md). For AI CLI behavior and automation rules, see [AGENT_CLI.md](AGENT_CLI.md).

The MCP server is the integration surface for agents. It does not replace the CLI: indexing, watching, daemon refresh, evidence collection, proof reports, and enterprise-proof reports still run through `callsieve` commands.

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
callsieve agent-setup /path/to/repo --client cursor
callsieve agent-setup /path/to/repo --client cline
callsieve agent-setup /path/to/repo --client roo
callsieve agent-setup /path/to/repo --client generic
```

Pass `--force` to replace existing generated files.
Generated MCP configs use the resolved CallSieve executable path so client startup does not depend on the agent shell PATH. Generated policy files also include the first command agents should run for every task: `callsieve agent-context <repo> "<task>"`. Manual examples below use `callsieve` for readability; replace it with an absolute path when the client shell cannot resolve the binary.

Audit generated setup with:

```bash
callsieve enforce /path/to/repo --client codex
callsieve enforce /path/to/repo --client codex --trace /path/to/trace.json --strict
```

`enforce` checks index freshness, generated client policy/config files, optional trace policy, and shim state. Missing shims are a warning unless `--require-shim` is passed.

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

## Claude Desktop

Claude Desktop's current local MCP path is desktop extensions. For broad Desktop distribution, package CallSieve as an `.mcpb` extension that launches `callsieve mcp`.

Until that package exists, use Claude Code or another stdio MCP client for direct local CallSieve access.

Reference: https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop

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

## Roo

Roo can use the generated `.roo/mcp.json` and `.roo/rules/callsieve.md` files:

```bash
callsieve agent-setup /path/to/repo --client roo
```

Use `callsieve_context` before broad search tools and repeated file reads.

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

## Grep Shims

For opt-in PATH-level interception, install local wrappers:

```bash
callsieve shim install /path/to/repo --force --strict
callsieve shim doctor /path/to/repo
```

Then prepend `/path/to/repo/.callsieve/bin` to the PATH used by the agent shell for that process. The install writes a local `callsieve` launcher plus `rg` and `grep` wrappers. The search wrappers call `callsieve grep` before passing through to the real `rg` or `grep` command captured during install. With `--strict`, shim-mediated grep writes `.callsieve/shim-trace.json` for strict trace audits. Remove wrappers with:

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
