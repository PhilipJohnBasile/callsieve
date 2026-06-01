# MCP Setup

`callsieve mcp` runs a local stdio MCP server. It exposes:

- `callsieve_context`
- `callsieve_symbol`
- `callsieve_stats`
- `callsieve_benchmark`

Build or install CallSieve first, then index each repository before using the MCP tools:

```bash
cargo install --path .
callsieve index /path/to/repo
callsieve mcp
```

If you do not install the binary, replace `callsieve` in the examples with:

```bash
cargo run --manifest-path /path/to/callsieve/Cargo.toml -- mcp
```

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

Use the `callsieve_context` tool before broad grep or repeated file reads.

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

## Tool Workflow

For coding tasks, agents should:

1. Call `callsieve_context` with `{ "path": "/path/to/repo", "task": "..." }`.
2. Read the returned `read_first` snippets and files.
3. Use `callsieve_symbol` for named symbols when needed.
4. Grep only when the context packet is insufficient.
