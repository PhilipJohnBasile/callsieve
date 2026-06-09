import * as vscode from "vscode";
import * as path from "path";
import {
  CallSieveClient,
  ContextPacket,
  LocalExpansion,
  ReadFirstFile,
  SelectionSummary,
} from "./client";

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
  lines.push(`Retrieval model tokens: ${packet.retrievalModelTokens ?? 0}`);
  if (typeof packet.estimatedTokens === "number") {
    const budget = typeof packet.tokenBudget === "number" ? ` / ${packet.tokenBudget}` : "";
    lines.push(`Estimated packet tokens: ${packet.estimatedTokens}${budget}`);
  }
  if (packet.localWork) {
    const files = packet.localWork.indexedFiles ?? 0;
    const symbols = packet.localWork.indexedSymbols ?? 0;
    const references = packet.localWork.indexedReferences ?? 0;
    lines.push(`Local work: ${files} files, ${symbols} symbols, ${references} references`);
  }
  appendSelectionMarkdown(lines, packet.selectionSummary);
  lines.push("");
  for (const file of packet.readFirst) {
    const sym = file.topSymbol ? ` - \`${file.topSymbol}\`` : "";
    const score = typeof file.score === "number" ? ` (score ${file.score})` : "";
    const graph = graphHintsMarkdown(file);
    const calls = callPathsMarkdown(file);
    lines.push(`- \`${file.file}\`${score}${sym}${graph}${calls}`);
  }
  appendExpansionMarkdown(lines, packet.localExpansion);
  lines.push("");
  return lines.join("\n");
}

function graphHintsMarkdown(file: ReadFirstFile): string {
  const parts: string[] = [];
  if (file.graphHints?.upstream?.length) {
    parts.push(`upstream: ${file.graphHints.upstream.map((path) => `\`${path}\``).join(", ")}`);
  }
  if (file.graphHints?.downstream?.length) {
    parts.push(`downstream: ${file.graphHints.downstream.map((path) => `\`${path}\``).join(", ")}`);
  }
  return parts.length ? ` - ${parts.join("; ")}` : "";
}

function callPathsMarkdown(file: ReadFirstFile): string {
  const parts: string[] = [];
  for (const edge of file.callPaths?.calls ?? []) {
    const relation = edge.from ? `${edge.from}->${edge.to}` : edge.to;
    const line = typeof edge.line === "number" ? `:${edge.line}` : "";
    parts.push(`calls ${relation} in \`${edge.file}${line}\``);
  }
  for (const edge of file.callPaths?.calledBy ?? []) {
    const relation = edge.from ? `${edge.from}->${edge.to}` : edge.to;
    const line = typeof edge.line === "number" ? `:${edge.line}` : "";
    parts.push(`called_by ${relation} in \`${edge.file}${line}\``);
  }
  return parts.length ? ` - ${parts.join("; ")}` : "";
}

function appendSelectionMarkdown(lines: string[], summary: SelectionSummary | undefined): void {
  if (!summary?.topFile) {
    return;
  }
  const score = typeof summary.topScore === "number" ? `, score ${summary.topScore}` : "";
  lines.push(`Selection: \`${summary.topFile}\`${score}`);
  if (summary.topSignals?.length) {
    const components = summary.topSignals
      .map((component) => typeof component.points === "number"
        ? `${component.name} +${component.points}`
        : component.name)
      .join(", ");
    lines.push(`Top local signals: ${components}`);
  }
  if (summary.topReason) {
    lines.push(`Why: ${summary.topReason}`);
  }
  if (summary.nextFiles?.length) {
    lines.push("Next ranked files:");
    for (const file of summary.nextFiles) {
      const score = typeof file.score === "number" ? `, score ${file.score}` : "";
      const reason = file.reason ? ` - ${file.reason}` : "";
      lines.push(`- \`${file.file}\`${score}${reason}`);
    }
  }
}

function appendExpansionMarkdown(lines: string[], expansion: LocalExpansion | undefined): void {
  if (!expansion) {
    return;
  }
  const commands = [
    expansion.inspectTopFile,
    ...(expansion.inspectNextFiles ?? []),
    expansion.expandRelationships,
    expansion.inspectTests,
    expansion.grepFallback,
  ].filter((command): command is string => Boolean(command));
  if (commands.length === 0) {
    return;
  }
  lines.push("");
  lines.push("## Local expansion before grep");
  for (const command of commands) {
    lines.push(`- \`${command}\``);
  }
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
    const limit = vscode.workspace.getConfiguration("callsieve").get<number>("limit", 5);
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
        estimatedTokens: packet.estimatedTokens,
        tokenBudget: packet.tokenBudget,
        retrievalModelTokens: packet.retrievalModelTokens,
        localWork: packet.localWork,
        selectionSummary: packet.selectionSummary,
        localExpansion: packet.localExpansion,
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
  #budget { font-size: 11px; opacity: 0.75; margin: 4px 0 8px; }
  #selection { font-size: 11px; margin: 4px 0 8px; opacity: 0.85; }
  #selection .signals { font-family: var(--vscode-editor-font-family); opacity: 0.8; padding-top: 2px; }
  #expansion { font-size: 11px; margin: 4px 0 8px; }
  #expansion .cmd { font-family: var(--vscode-editor-font-family); opacity: 0.85; padding: 2px 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
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
  <div id="budget"></div>
  <div id="selection"></div>
  <div id="expansion"></div>
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
    const budgetEl = document.getElementById('budget');
    const selectionEl = document.getElementById('selection');
    const expansionEl = document.getElementById('expansion');
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
        const estimated = msg.packet.estimatedTokens;
        const budget = msg.packet.tokenBudget;
        const retrieval = msg.packet.retrievalModelTokens ?? 0;
        const localWork = msg.packet.localWork;
        const local = localWork
          ? ' · local ' + (localWork.indexedFiles ?? 0) + ' files, ' + (localWork.indexedSymbols ?? 0) + ' symbols, ' + (localWork.indexedReferences ?? 0) + ' refs'
          : '';
        budgetEl.textContent = typeof estimated === 'number'
          ? 'retrieval ' + retrieval + ' model tokens · packet ' + estimated + (typeof budget === 'number' ? '/' + budget : '') + ' est. tokens' + local
          : 'retrieval ' + retrieval + ' model tokens' + local;
        selectionEl.innerHTML = '';
        const selection = msg.packet.selectionSummary;
        if (selection && selection.topFile) {
          const title = document.createElement('div');
          title.textContent = 'Selected ' + selection.topFile + (typeof selection.topScore === 'number' ? ' · score ' + selection.topScore : '');
          selectionEl.appendChild(title);
          const components = selection.topSignals || [];
          if (components.length) {
            const signals = document.createElement('div');
            signals.className = 'signals';
            signals.textContent = components.map((component) => component.name + ' +' + component.points).join(', ');
            selectionEl.appendChild(signals);
          }
          if (selection.topReason) {
            const why = document.createElement('div');
            why.className = 'signals';
            why.textContent = selection.topReason;
            selectionEl.appendChild(why);
          }
          const nextFiles = selection.nextFiles || [];
          if (nextFiles.length) {
            const next = document.createElement('div');
            next.className = 'signals';
            next.textContent = 'Next: ' + nextFiles.map((file) => file.file + (typeof file.score === 'number' ? ' · ' + file.score : '')).join(', ');
            selectionEl.appendChild(next);
          }
        }
        expansionEl.innerHTML = '';
        const expansion = msg.packet.localExpansion || {};
        const commands = [
          expansion.inspectTopFile,
          ...(expansion.inspectNextFiles || []),
          expansion.expandRelationships,
          expansion.inspectTests,
          expansion.grepFallback,
        ].filter(Boolean);
        if (commands.length) {
          const title = document.createElement('div');
          title.textContent = 'Local expansion before grep';
          expansionEl.appendChild(title);
          for (const command of commands) {
            const div = document.createElement('div');
            div.className = 'cmd';
            div.title = command;
            div.textContent = command;
            expansionEl.appendChild(div);
          }
        }
        emptyEl.style.display = files.length ? 'none' : 'block';
        for (const f of files) {
          const div = document.createElement('div');
          div.className = 'file';
          const pathEl = document.createElement('div');
          pathEl.className = 'path';
          pathEl.textContent = f.file;
          const metaEl = document.createElement('div');
          metaEl.className = 'meta';
          const graphParts = [];
          if (f.graphHints && f.graphHints.upstream && f.graphHints.upstream.length) {
            graphParts.push('up ' + f.graphHints.upstream.join(', '));
          }
          if (f.graphHints && f.graphHints.downstream && f.graphHints.downstream.length) {
            graphParts.push('down ' + f.graphHints.downstream.join(', '));
          }
          metaEl.textContent = 'score ' + f.score
            + (f.topSymbol ? ' - ' + f.topSymbol : '')
            + (graphParts.length ? ' - ' + graphParts.join(' | ') : '');
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
