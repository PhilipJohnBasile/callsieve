# CallSieve for VS Code

A minimal VS Code sidebar that shows you the read-first set of files for a
coding task, computed locally by the [CallSieve](https://github.com/PhilipJohnBasile/callsieve)
CLI. No tokens spent on retrieval; no network calls.

This is the compact editor slice of CallSieve: one input box, a live list of
files, visible token-accounting hints, local selection signals, capped graph
hints, local expansion commands, and a "Copy as Markdown" command. The Cursor variant, "Send to Claude
Code" buttons, telemetry, and freshness indicators all come in follow-up tasks.

## Requirements

- The `callsieve` binary must be installed. Either:
  - On your `PATH` (recommended).
  - Or set `callsieve.binaryPath` in VS Code settings to an absolute path.
  - Or place it at `<workspace>/.callsieve/bin/callsieve`.
- A workspace folder open in VS Code with a built CallSieve index
  (`callsieve index .` in the workspace root). If the index is stale the
  extension will trigger a rebuild on the first query.

## Install from VSIX

```bash
cd editors/vscode
npm install
npm run compile
# Package and install (requires vsce):
npx @vscode/vsce package
code --install-extension callsieve-vscode-*.vsix
```

For local development you can skip packaging and load the unpacked extension
straight from the source tree:

```bash
code --extensionDevelopmentPath=editors/vscode
```

## Usage

1. Open the CallSieve view from the activity bar (search-style icon).
2. Type a task in the input box. The list of read-first files updates after
   a 250 ms debounce.
3. Check the token line to confirm retrieval used zero AI model tokens, see
   the estimated packet size against the budget, and see the local files,
   symbols, and references CallSieve searched on-machine.
4. Check the selection line to see the top local ranking signals behind the
   first file and the next ranked files when CallSieve returns them. Skim
   packets can also include one upstream/downstream non-test graph preview;
   use local expansion commands for call-path detail.
5. Use the local expansion commands shown under the token line before broad
   grep when the read-first packet is insufficient. Focus commands include a
   symbol when CallSieve selected one.
6. Click any file to open it in the editor.
7. Run the `CallSieve: Copy as Markdown` command (Cmd/Ctrl-Shift-P) to copy
   the current packet, token accounting, selection signals, next ranked files,
   and local expansion commands. Paste it into Claude Code, Cursor's composer,
   Copilot Chat, or any other agent prompt.

A small status line under the input shows whether the extension is talking
to the long-lived `callsieve mcp` subprocess (`MCP`) or shelling out per
query (`CLI fallback`). The fallback engages automatically if `callsieve mcp`
fails to start or dies mid-session.

## Configuration

| Setting | Default | Meaning |
| --- | --- | --- |
| `callsieve.binaryPath` | `""` | Absolute path to the `callsieve` binary. Empty means: try PATH, then `<workspace>/.callsieve/bin/callsieve`. |
| `callsieve.limit` | `5` | Number of read-first files to request. |
| `callsieve.debounceMs` | `250` | Milliseconds to wait after the last keystroke before sending the task. |

## What this extension does NOT do (yet)

- No "Send to Claude Code", "Send to Cursor composer", or "Copy as Copilot
  prompt" buttons. Just "Copy as Markdown".
- No telemetry, no `first_correct_file_rate` capture, no team dashboard
  hook.
- No Cursor publishing manifest. Cursor support is a separate task.
- No freshness/daemon status bar. The MCP server will auto-rebuild a stale
  index on the first query; subsequent slice work adds the indicator.

## Building

```bash
npm install
npm run compile
```

`npm run compile` runs the TypeScript compiler. There are no esbuild or
bundler steps - the extension is small enough that plain `tsc` keeps the
toolchain minimal.

## Smoke test

```bash
npm run compile
npm test
```

The test uses `@vscode/test-electron` to launch a disposable VS Code
instance, activate the extension, and verify the `CallSieve` view is
registered. It does not exercise the CallSieve binary itself.
