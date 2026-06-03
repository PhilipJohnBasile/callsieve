CallSieve policy for claude:
1. First command for every coding task: `callsieve agent-context . "<task>"`.
2. Use the project MCP server from `.mcp.json` and prefer `callsieve_context` when tools are available.
3. Read the returned `read_first` files before broad grep, rg, repository-wide search, or repeated file reads.
4. grep only if the context packet is insufficient, and preserve why it was insufficient in the task notes or trace.
5. For audited sessions, run `callsieve enforce <repo> --client claude --trace <trace.json> --strict`.
