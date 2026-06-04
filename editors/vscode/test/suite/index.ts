import * as assert from "assert";
import * as vscode from "vscode";

// Plain-JS test runner - no Mocha dependency. The VS Code test harness loads
// this module's `run` export and awaits its result. Throwing or rejecting
// fails the test run.
export async function run(): Promise<void> {
  await activateExtension();

  // The extension contributes a view container called "callsieve" and a
  // webview view called "callsieve.sidebar". The view container shows up in
  // the activity bar; we can't introspect the activity bar directly via
  // public API, but we can verify the registered command.
  const commands = await vscode.commands.getCommands(true);
  assert.ok(
    commands.includes("callsieve.copyAsMarkdown"),
    "callsieve.copyAsMarkdown command must be registered",
  );

  console.log("[callsieve test] smoke OK");
}

async function activateExtension(): Promise<void> {
  // The publisher.name from package.json is callsieve.callsieve-vscode.
  const ext = vscode.extensions.getExtension("callsieve.callsieve-vscode");
  assert.ok(ext, "callsieve extension must be discoverable");
  if (!ext.isActive) {
    await ext.activate();
  }
}
