import * as vscode from "vscode";
import * as path from "path";
import { CallSieveClient, ContextPacket, ReadFirstFile } from "./client";

let currentClient: CallSieveClient | undefined;
let currentPacket: ContextPacket | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const binaryPath = resolveBinaryPath();
  currentClient = new CallSieveClient(binaryPath);
  await currentClient.start();

  const provider = new CallSieveViewProvider(context.extensionUri, currentClient);
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider("callsieve.sidebar", provider, {
      webviewOptions: { retainContextWhenHidden: true },
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("callsieve.copyAsMarkdown", async () => {
      if (!currentPacket || currentPacket.readFirst.length === 0) {
        vscode.window.showInformationMessage(
          "CallSieve: no read-first packet yet. Type a task in the sidebar first.",
        );
        return;
      }
      const md = packetToMarkdown(currentPacket);
      await vscode.env.clipboard.writeText(md);
      vscode.window.showInformationMessage("CallSieve: packet copied as Markdown.");
    }),
  );

  context.subscriptions.push({
    dispose: () => {
      currentClient?.dispose();
    },
  });
}

export function deactivate(): void {
  currentClient?.dispose();
  currentClient = undefined;
}

function resolveBinaryPath(): string {
  const configured = vscode.workspace.getConfiguration("callsieve").get<string>("binaryPath");
  if (configured && configured.trim().length > 0) {
    return configured;
  }
  // Try workspace-relative .callsieve/bin/callsieve as a final fallback. We
  // return "callsieve" by default and let the client try PATH first; if the
  // PATH lookup fails, the client will retry with the workspace-relative
  // fallback path.
  return "callsieve";
}

function packetToMarkdown(packet: ContextPacket): string {
  const lines: string[] = [];
  lines.push(`# CallSieve read-first for: ${packet.task}`);
  lines.push("");
  for (const file of packet.readFirst) {
    const sym = file.topSymbol ? ` - \`${file.topSymbol}\`` : "";
    lines.push(`- \`${file.file}\` (score ${file.score})${sym}`);
  }
  lines.push("");
  return lines.join("\n");
}

class CallSieveViewProvider implements vscode.WebviewViewProvider {
  private view: vscode.WebviewView | undefined;
  private debounceHandle: NodeJS.Timeout | undefined;
  private lastTask = "";

  constructor(
    private readonly extensionUri: vscode.Uri,
    private readonly client: CallSieveClient,
  ) {}

  public resolveWebviewView(webviewView: vscode.WebviewView): void {
    this.view = webviewView;
    webviewView.webview.options = {
      enableScripts: true,
      localResourceRoots: [this.extensionUri],
    };
    webviewView.webview.html = this.getHtml();
    this.postMode();

    webviewView.webview.onDidReceiveMessage(async (msg: WebviewInbound) => {
      if (msg.type === "task") {
        this.scheduleTask(msg.value);
      } else if (msg.type === "open") {
        await openWorkspaceFile(msg.file);
      } else if (msg.type === "copy") {
        await vscode.commands.executeCommand("callsieve.copyAsMarkdown");
      } else if (msg.type === "ready") {
        this.postMode();
      }
    });
  }

  private scheduleTask(value: string): void {
    this.lastTask = value;
    if (this.debounceHandle) {
      clearTimeout(this.debounceHandle);
    }
    const debounce = vscode.workspace
      .getConfiguration("callsieve")
      .get<number>("debounceMs", 250);
    this.debounceHandle = setTimeout(() => {
      void this.runTask(value);
    }, debounce);
  }

  private async runTask(task: string): Promise<void> {
    const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (!root) {
      this.postError("No workspace folder open.");
      return;
    }
    if (!task.trim()) {
      this.postResults({ task, readFirst: [], root });
      return;
    }
    const limit = vscode.workspace.getConfiguration("callsieve").get<number>("limit", 8);
    try {
      const packet = await this.client.context(root, task, limit);
      // Drop stale results if the user kept typing.
      if (task !== this.lastTask) {
        return;
      }
      currentPacket = packet;
      this.postResults(packet);
      this.postMode();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.postError(message);
    }
  }

  private postResults(packet: ContextPacket): void {
    this.view?.webview.postMessage({
      type: "results",
      packet: {
        task: packet.task,
        readFirst: packet.readFirst,
      },
    });
  }

  private postMode(): void {
    this.view?.webview.postMessage({
      type: "mode",
      mode: this.client.mode(),
    });
  }

  private postError(message: string): void {
    this.view?.webview.postMessage({ type: "error", message });
  }

  private getHtml(): string {
    // Self-contained webview HTML. No external resources.
    return /* html */ `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<style>
  body { font-family: var(--vscode-font-family); color: var(--vscode-foreground); padding: 8px; }
  input { width: 100%; box-sizing: border-box; padding: 6px; background: var(--vscode-input-background); color: var(--vscode-input-foreground); border: 1px solid var(--vscode-input-border, transparent); }
  #status { font-size: 11px; opacity: 0.7; margin: 6px 0; display: flex; justify-content: space-between; }
  .file { padding: 6px 4px; border-bottom: 1px solid var(--vscode-editorWidget-border, rgba(128,128,128,0.2)); cursor: pointer; }
  .file:hover { background: var(--vscode-list-hoverBackground); }
  .file .path { font-family: var(--vscode-editor-font-family); font-size: 12px; }
  .file .meta { font-size: 11px; opacity: 0.7; }
  #error { color: var(--vscode-errorForeground); font-size: 12px; margin-top: 8px; white-space: pre-wrap; }
  #copy { margin-top: 8px; padding: 4px 8px; background: var(--vscode-button-background); color: var(--vscode-button-foreground); border: none; cursor: pointer; }
  #copy:hover { background: var(--vscode-button-hoverBackground); }
  #empty { font-size: 12px; opacity: 0.6; padding: 8px 0; }
</style>
</head>
<body>
  <input id="task" type="text" placeholder="Describe a task..." aria-label="Task" />
  <div id="status"><span id="mode">starting...</span><span id="count"></span></div>
  <div id="files"></div>
  <div id="empty">Type a task above to see read-first files.</div>
  <button id="copy" type="button">Copy as Markdown</button>
  <div id="error"></div>
  <script>
    const vscode = acquireVsCodeApi();
    const taskEl = document.getElementById('task');
    const filesEl = document.getElementById('files');
    const emptyEl = document.getElementById('empty');
    const modeEl = document.getElementById('mode');
    const countEl = document.getElementById('count');
    const errorEl = document.getElementById('error');
    const copyBtn = document.getElementById('copy');

    taskEl.addEventListener('input', () => {
      errorEl.textContent = '';
      vscode.postMessage({ type: 'task', value: taskEl.value });
    });
    copyBtn.addEventListener('click', () => {
      vscode.postMessage({ type: 'copy' });
    });

    window.addEventListener('message', (event) => {
      const msg = event.data;
      if (msg.type === 'results') {
        filesEl.innerHTML = '';
        const files = msg.packet.readFirst || [];
        countEl.textContent = files.length ? files.length + ' files' : '';
        emptyEl.style.display = files.length ? 'none' : 'block';
        for (const f of files) {
          const div = document.createElement('div');
          div.className = 'file';
          const pathEl = document.createElement('div');
          pathEl.className = 'path';
          pathEl.textContent = f.file;
          const metaEl = document.createElement('div');
          metaEl.className = 'meta';
          metaEl.textContent = 'score ' + f.score + (f.topSymbol ? ' - ' + f.topSymbol : '');
          div.appendChild(pathEl);
          div.appendChild(metaEl);
          div.addEventListener('click', () => {
            vscode.postMessage({ type: 'open', file: f.file });
          });
          filesEl.appendChild(div);
        }
      } else if (msg.type === 'mode') {
        modeEl.textContent = msg.mode;
      } else if (msg.type === 'error') {
        errorEl.textContent = msg.message;
      }
    });

    vscode.postMessage({ type: 'ready' });
  </script>
</body>
</html>`;
  }
}

interface WebviewInboundTask {
  type: "task";
  value: string;
}
interface WebviewInboundOpen {
  type: "open";
  file: string;
}
interface WebviewInboundCopy {
  type: "copy";
}
interface WebviewInboundReady {
  type: "ready";
}

type WebviewInbound =
  | WebviewInboundTask
  | WebviewInboundOpen
  | WebviewInboundCopy
  | WebviewInboundReady;

async function openWorkspaceFile(relPath: string): Promise<void> {
  const root = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  if (!root) {
    return;
  }
  const absolute = path.isAbsolute(relPath) ? relPath : path.join(root, relPath);
  const uri = vscode.Uri.file(absolute);
  await vscode.window.showTextDocument(uri, { preview: true });
}

export { ReadFirstFile };
