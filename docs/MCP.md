# MCP Setup

`callsieve mcp` runs a local stdio MCP server. It exposes:

- `callsieve_context`
- `callsieve_symbol`
- `callsieve_stats`
- `callsieve_status`
- `callsieve_trace_check`
- `callsieve_benchmark`

Build or install CallSieve first, then index each repository before using the MCP tools:

```bash
cargo install --path .
callsieve index /path/to/repo
callsieve mcp
```

For higher-confidence reference edges, index with local LSP enrichment before starting MCP:

```bash
callsieve index /path/to/repo --lsp
```

The MCP server reads the existing `.callsieve/index.json`; it does not install language servers or rebuild indexes.

If you do not install the binary, replace `callsieve` in the examples with:

```bash
cargo run --manifest-path /path/to/callsieve/Cargo.toml -- mcp
```

CallSieve can generate local config and a before-grep policy file for supported clients:

```bash
callsieve agent-setup /path/to/repo --client codex
callsieve agent-setup /path/to/repo --client claude
callsieve agent-setup /path/to/repo --client cursor
callsieve agent-setup /path/to/repo --client cline
callsieve agent-setup /path/to/repo --client roo
callsieve agent-setup /path/to/repo --client generic
```

Pass `--force` to replace existing generated files.

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
command = "callsieve"
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

Use a different `--model` label for each Codex/ChatGPT model you test. CallSieve records and audits those sessions; it does not invoke hidden ChatGPT models itself.

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

1. Call `callsieve_context` with `{ "path": "/path/to/repo", "task": "..." }`.
2. Call `callsieve_status` if freshness or LSP enrichment state is uncertain.
3. Read the returned `read_first` snippets and files.
4. Use `callsieve_symbol` for named symbols when needed.
5. Grep only when the context packet is insufficient.

The `callsieve_context` tool metadata marks it as the preferred first tool for codebase discovery. Its practical instruction is: read these files first; grep only if insufficient.

Use `callsieve_trace_check` on captured trace JSON to detect sessions that ran grep before CallSieve. Pass `"strict": true` to also fail common file reads before `callsieve_context`.

## Grep Shims

For opt-in PATH-level interception, install local wrappers:

```bash
callsieve shim install /path/to/repo --force
callsieve shim doctor /path/to/repo
```

Then prepend `/path/to/repo/.callsieve/bin` to the PATH used by the agent shell. The wrappers call `callsieve grep` before passing through to the real `rg` or `grep` command captured during install. Remove them with:

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
