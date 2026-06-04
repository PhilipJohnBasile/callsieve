CallSieve policy for codex:
1. First command for every coding task: `callsieve agent-context . "<task>"`.
2. Use the configured Codex MCP server and prefer `callsieve_context` when tools are available.
3. Treat `retrieval_cost.retrieval_model_tokens = 0` as retrieval-only; returned context still counts when read.
4. Read the returned `read_first` files before broad grep, rg, repository-wide search, or repeated file reads.
5. Use `callsieve_focus`, `callsieve_related`, or `callsieve_tests` for targeted detail before broad search.
6. Grep only if the context packet is insufficient, and preserve why it was insufficient in the task notes or trace.
7. For audited sessions, run `callsieve enforce <repo> --client codex --trace <trace.json> --strict`.
