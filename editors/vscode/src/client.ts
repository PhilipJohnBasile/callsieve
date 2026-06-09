import { spawn, ChildProcessWithoutNullStreams, spawnSync } from "child_process";
import * as path from "path";
import * as fs from "fs";
import * as vscode from "vscode";

export interface ReadFirstFile {
  file: string;
  score?: number;
  topSymbol?: string;
  topSymbolLine?: number;
  graphHints?: GraphHints;
  callPaths?: CallPaths;
}

export interface GraphHints {
  upstream?: string[];
  downstream?: string[];
}

export interface CallPaths {
  calls?: CallPathEdge[];
  calledBy?: CallPathEdge[];
}

export interface CallPathEdge {
  file: string;
  to: string;
  from?: string;
  line?: number;
}

export interface ContextPacket {
  task: string;
  root: string;
  readFirst: ReadFirstFile[];
  estimatedTokens?: number;
  tokenBudget?: number;
  retrievalModelTokens?: number;
  localWork?: LocalWork;
  selectionSummary?: SelectionSummary;
  localExpansion?: LocalExpansion;
}

export interface LocalWork {
  indexedFiles?: number;
  indexedSymbols?: number;
  indexedReferences?: number;
}

export interface SelectionSummary {
  topFile?: string;
  topScore?: number;
  topReason?: string;
  topSignals?: SelectionScoreComponent[];
  nextFiles?: SelectionNextFile[];
}

export interface SelectionNextFile {
  file: string;
  score?: number;
  reason?: string;
}

export interface SelectionScoreComponent {
  name: string;
  points?: number;
}

export interface LocalExpansion {
  policy?: string;
  inspectTopFile?: string;
  inspectNextFiles?: string[];
  expandRelationships?: string;
  inspectTests?: string;
  grepFallback?: string;
}

export type ClientMode = "MCP" | "CLI fallback" | "starting" | "unavailable";

/**
 * CallSieveClient owns the long-lived `callsieve mcp` subprocess and falls back
 * to per-query `callsieve agent-context --format json` if MCP fails.
 *
 * MCP requests are serialized: we keep a single in-flight request at a time.
 * The sidebar debounces input, so contention here is rare.
 */
export class CallSieveClient {
  private proc: ChildProcessWithoutNullStreams | undefined;
  private buffer = "";
  private nextId = 1;
  private pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: Error) => void }
  >();
  private resolvedBinary: string | undefined;
  private currentMode: ClientMode = "starting";
  private initialized = false;

  constructor(private readonly configuredBinary: string) {}

  public mode(): ClientMode {
    return this.currentMode;
  }

  public async start(): Promise<void> {
    const binary = this.resolveBinary();
    if (!binary) {
      this.currentMode = "unavailable";
      return;
    }
    this.resolvedBinary = binary;
    try {
      await this.startMcp(binary);
      this.currentMode = "MCP";
    } catch (err) {
      console.warn("[callsieve] MCP failed to start, will use CLI fallback:", err);
      this.currentMode = "CLI fallback";
    }
  }

  private resolveBinary(): string | undefined {
    // 1. Configured absolute path.
    if (this.configuredBinary && path.isAbsolute(this.configuredBinary)) {
      if (fs.existsSync(this.configuredBinary)) {
        return this.configuredBinary;
      }
    }
    // 2. Lookup on PATH (callsieve or configured short name).
    const candidate = this.configuredBinary || "callsieve";
    if (this.binaryOnPath(candidate)) {
      return candidate;
    }
    // 3. Workspace-relative .callsieve/bin/callsieve fallback.
    const folder = vscode.workspace.workspaceFolders?.[0];
    if (folder) {
      const local = path.join(folder.uri.fsPath, ".callsieve", "bin", "callsieve");
      if (fs.existsSync(local)) {
        return local;
      }
    }
    return undefined;
  }

  private binaryOnPath(name: string): boolean {
    if (path.isAbsolute(name)) {
      return fs.existsSync(name);
    }
    // Use `which`/`where` via spawnSync.
    const cmd = process.platform === "win32" ? "where" : "which";
    try {
      const result = spawnSync(cmd, [name], { encoding: "utf8" });
      return result.status === 0 && result.stdout.trim().length > 0;
    } catch {
      return false;
    }
  }

  private async startMcp(binary: string): Promise<void> {
    const proc = spawn(binary, ["mcp"], {
      stdio: ["pipe", "pipe", "pipe"],
    });
    this.proc = proc;
    proc.stdout.setEncoding("utf8");
    proc.stderr.setEncoding("utf8");
    proc.stdout.on("data", (chunk: string) => this.onStdout(chunk));
    proc.stderr.on("data", (chunk: string) => {
      console.warn("[callsieve mcp stderr]", chunk);
    });
    proc.on("exit", (code) => {
      console.warn("[callsieve mcp] exited with code", code);
      this.failAllPending(new Error("callsieve mcp exited"));
      this.proc = undefined;
      this.initialized = false;
      // If the process dies during a session, drop to CLI fallback.
      this.currentMode = "CLI fallback";
    });
    proc.on("error", (err) => {
      console.warn("[callsieve mcp] error:", err);
    });
    // Initialize.
    await this.rpc("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "callsieve-vscode", version: "0.1.0" },
    });
    this.initialized = true;
  }

  private onStdout(chunk: string): void {
    this.buffer += chunk;
    let newlineIdx: number;
    while ((newlineIdx = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, newlineIdx).trim();
      this.buffer = this.buffer.slice(newlineIdx + 1);
      if (!line) {
        continue;
      }
      this.handleLine(line);
    }
  }

  private handleLine(line: string): void {
    let parsed: unknown;
    try {
      parsed = JSON.parse(line);
    } catch {
      console.warn("[callsieve mcp] non-JSON line:", line);
      return;
    }
    if (!parsed || typeof parsed !== "object") {
      return;
    }
    const msg = parsed as { id?: number; result?: unknown; error?: { message?: string } };
    if (typeof msg.id !== "number") {
      return;
    }
    const entry = this.pending.get(msg.id);
    if (!entry) {
      return;
    }
    this.pending.delete(msg.id);
    if (msg.error) {
      entry.reject(new Error(msg.error.message ?? "MCP error"));
      return;
    }
    entry.resolve(msg.result);
  }

  private rpc(method: string, params: unknown): Promise<unknown> {
    const proc = this.proc;
    if (!proc) {
      return Promise.reject(new Error("callsieve mcp is not running"));
    }
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      const payload = JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n";
      try {
        proc.stdin.write(payload);
      } catch (err) {
        this.pending.delete(id);
        reject(err instanceof Error ? err : new Error(String(err)));
        return;
      }
      // Soft timeout so the sidebar doesn't hang forever on a wedged child.
      setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id);
          reject(new Error(`callsieve mcp ${method} timed out`));
        }
      }, 30_000);
    });
  }

  public async context(root: string, task: string, limit: number): Promise<ContextPacket> {
    if (this.currentMode === "MCP" && this.proc && this.initialized) {
      try {
        const result = (await this.rpc("tools/call", {
          name: "callsieve_context",
          arguments: { path: root, task, limit },
        })) as McpToolResult;
        return parseStructured(result, task, root);
      } catch (err) {
        console.warn("[callsieve mcp] tool call failed, falling back to CLI:", err);
        this.currentMode = "CLI fallback";
      }
    }
    // CLI fallback path.
    return this.contextViaCli(root, task, limit);
  }

  private async contextViaCli(root: string, task: string, limit: number): Promise<ContextPacket> {
    const binary = this.resolvedBinary || this.resolveBinary();
    if (!binary) {
      throw new Error(
        "CallSieve binary not found. Set callsieve.binaryPath or install callsieve on PATH.",
      );
    }
    this.resolvedBinary = binary;
    return new Promise((resolve, reject) => {
      const proc = spawn(
        binary,
        ["agent-context", root, task, "--format", "json", "--limit", String(limit)],
        { stdio: ["ignore", "pipe", "pipe"] },
      );
      let stdout = "";
      let stderr = "";
      proc.stdout.setEncoding("utf8");
      proc.stderr.setEncoding("utf8");
      proc.stdout.on("data", (chunk: string) => (stdout += chunk));
      proc.stderr.on("data", (chunk: string) => (stderr += chunk));
      proc.on("error", (err) => reject(err));
      proc.on("close", (code) => {
        if (code !== 0) {
          reject(new Error(`callsieve agent-context exited ${code}: ${stderr.trim()}`));
          return;
        }
        try {
          const parsed = JSON.parse(stdout);
          resolve(parseAgentContextOutput(parsed, task, root));
        } catch (err) {
          reject(err instanceof Error ? err : new Error(String(err)));
        }
      });
    });
  }

  private failAllPending(error: Error): void {
    for (const entry of this.pending.values()) {
      entry.reject(error);
    }
    this.pending.clear();
  }

  public dispose(): void {
    this.failAllPending(new Error("client disposed"));
    if (this.proc) {
      try {
        this.proc.stdin.end();
      } catch {
        // ignore
      }
      this.proc.kill();
      this.proc = undefined;
    }
  }
}

interface McpToolResult {
  structuredContent?: unknown;
  isError?: boolean;
  content?: Array<{ type: string; text: string }>;
}

interface ReadFirstRaw {
  f?: string;
  file?: string;
  s?: number;
  score?: number;
  sy?: unknown[];
  symbols?: unknown[];
  g?: unknown;
  cp?: unknown;
  graph_hints?: unknown;
  call_paths?: unknown;
}

function parseStructured(result: McpToolResult, task: string, root: string): ContextPacket {
  if (result.isError) {
    const message = result.content?.find((c) => c.type === "text")?.text ?? "MCP returned error";
    throw new Error(message);
  }
  const structured = result.structuredContent;
  return extractPacket(structured, task, root);
}

function parseAgentContextOutput(parsed: unknown, task: string, root: string): ContextPacket {
  // The agent-context CLI wraps the read-first list inside `context`.
  if (parsed && typeof parsed === "object") {
    const ctx = (parsed as { context?: unknown }).context;
    const instruction = (parsed as { instruction?: unknown }).instruction;
    if (ctx) {
      return extractPacket(ctx, task, root, instruction);
    }
  }
  return extractPacket(parsed, task, root);
}

function extractPacket(
  structured: unknown,
  task: string,
  root: string,
  instructionOverride?: unknown,
): ContextPacket {
  const readFirst: ReadFirstFile[] = [];
  let estimatedTokens: number | undefined;
  let tokenBudget: number | undefined;
  let retrievalModelTokens: number | undefined;
  let localWork: LocalWork | undefined;
  let selectionSummary: SelectionSummary | undefined;
  let localExpansion: LocalExpansion | undefined;
  if (structured && typeof structured === "object") {
    const raw = structured as {
      read_first?: ReadFirstRaw[];
      stats?: {
        t?: unknown;
        estimated_tokens?: unknown;
        tokens?: unknown;
        b?: unknown;
        token_budget?: unknown;
        budget?: unknown;
        local_work?: unknown;
        local?: unknown;
      };
      retrieval_cost?: { retrieval_model_tokens?: unknown };
      selection_summary?: unknown;
      sel?: unknown;
      instruction?: unknown;
    };
    const arr = raw.read_first;
    const readFirstItems: ReadFirstRaw[] = [];
    if (Array.isArray(arr)) {
      for (const item of arr) {
        if (!item || typeof item !== "object") {
          continue;
        }
        const file = stringField(item.f) ?? stringField(item.file);
        if (!file) {
          continue;
        }
        const symbols = item.sy ?? item.symbols;
        const topSymbolEntry = Array.isArray(symbols) ? symbols[0] : undefined;
        const topSymbol = topSymbolEntry ? symbolName(topSymbolEntry) : undefined;
        const topSymbolLine = topSymbolEntry ? symbolLine(topSymbolEntry) : undefined;
        readFirstItems.push(item);
        readFirst.push({
          file,
          score: numberField(item.s) ?? numberField(item.score),
          topSymbol,
          topSymbolLine,
          graphHints: extractGraphHints(item.g ?? item.graph_hints),
        });
      }
      for (const [index, item] of readFirstItems.entries()) {
        const callPaths = extractCallPaths(item.cp ?? item.call_paths, readFirst);
        if (callPaths) {
          readFirst[index].callPaths = callPaths;
        }
      }
    }
    estimatedTokens = numberField(raw.stats?.t)
      ?? numberField(raw.stats?.tokens)
      ?? numberField(raw.stats?.estimated_tokens);
    tokenBudget = numberField(raw.stats?.b)
      ?? numberField(raw.stats?.budget)
      ?? numberField(raw.stats?.token_budget);
    retrievalModelTokens = numberField(raw.retrieval_cost?.retrieval_model_tokens);
    localWork = extractLocalWork(raw.stats?.local ?? raw.stats?.local_work);
    selectionSummary = extractSelectionSummary(raw.sel ?? raw.selection_summary, readFirst);
    localExpansion = extractLocalExpansion(instructionOverride ?? raw.instruction, root, readFirst);
  }
  return {
    task,
    root,
    readFirst,
    estimatedTokens,
    tokenBudget,
    retrievalModelTokens,
    localWork,
    selectionSummary,
    localExpansion,
  };
}

function numberField(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function extractLocalWork(value: unknown): LocalWork | undefined {
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  const localWork: LocalWork = {
    indexedFiles: numberField(raw.f) ?? numberField(raw.files) ?? numberField(raw.indexed_files),
    indexedSymbols: numberField(raw.sy) ?? numberField(raw.symbols) ?? numberField(raw.indexed_symbols),
    indexedReferences: numberField(raw.r) ?? numberField(raw.refs) ?? numberField(raw.indexed_references),
  };
  return Object.values(localWork).some((field) => typeof field === "number")
    ? localWork
    : undefined;
}

function symbolName(value: unknown): string | undefined {
  if (Array.isArray(value)) {
    return stringField(value[0]);
  }
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  return stringField(raw.n) ?? stringField(raw.name);
}

function symbolLine(value: unknown): number | undefined {
  if (Array.isArray(value)) {
    return numberField(value[1]);
  }
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  const lines = raw.ls ?? raw.lines;
  if (Array.isArray(lines)) {
    return numberField(lines[0]);
  }
  return numberField(raw.l) ?? numberField(raw.line);
}

function extractLocalExpansion(
  instruction: unknown,
  root: string,
  readFirst: ReadFirstFile[],
): LocalExpansion | undefined {
  if (!instruction || typeof instruction !== "object") {
    return undefined;
  }
  const expansion = (instruction as { x?: unknown; local_first_expansion?: unknown }).x
    ?? (instruction as { local_first_expansion?: unknown }).local_first_expansion;
  if (!expansion || typeof expansion !== "object") {
    return undefined;
  }
  const raw = expansion as Record<string, unknown>;
  const top = raw.o ?? raw.top ?? raw.inspect_top_file;
  const topFile = expansionTargetFile(top, readFirst);
  const localExpansion: LocalExpansion = {
    policy: stringField(raw.policy),
    inspectTopFile: expansionCommand(top, "focus", root, topFile, readFirst),
    inspectNextFiles: expansionCommands(raw.n ?? raw.next ?? raw.inspect_next_files, "focus", root, topFile, readFirst),
    expandRelationships: expansionCommand(raw.r ?? raw.rel ?? raw.expand_relationships, "related", root, topFile, readFirst),
    inspectTests: expansionCommand(raw.t ?? raw.tests ?? raw.inspect_tests, "tests", root, topFile, readFirst),
    grepFallback: expansionCommand(raw.grep ?? raw.grep_fallback, "grep", root, topFile, readFirst),
  };
  return Object.values(localExpansion).some((value) => Array.isArray(value) ? value.length > 0 : Boolean(value))
    ? localExpansion
    : undefined;
}

function extractSelectionSummary(
  value: unknown,
  readFirst: ReadFirstFile[],
): SelectionSummary | undefined {
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  const rawSignals = raw.sig ?? raw.top_signals;
  const components = Array.isArray(rawSignals)
    ? rawSignals
        .map((component) => {
          if (typeof component === "string") {
            const name = selectionSignalName(component);
            return name ? { name } : undefined;
          }
          if (Array.isArray(component)) {
            const name = selectionSignalName(stringField(component[0]));
            const points = numberField(component[1]);
            return name ? { name, points } : undefined;
          }
          if (!component || typeof component !== "object") {
            return undefined;
          }
          const entry = component as Record<string, unknown>;
          const name = selectionSignalName(stringField(entry.n) ?? stringField(entry.name));
          const points = numberField(entry.p) ?? numberField(entry.points);
          return name ? { name, points } : undefined;
        })
        .filter((component): component is SelectionScoreComponent => Boolean(component))
    : undefined;
  const compactTop = Array.isArray(raw.top) ? raw.top : undefined;
  const summary: SelectionSummary = {
    topFile: selectionFileRef(compactTop?.[0] ?? raw.top, readFirst) ?? stringField(raw.top_file),
    topScore: selectionEntryScore(compactTop, readFirst) ?? numberField(raw.s) ?? numberField(raw.top_score),
    topReason: selectionEntryReason(compactTop) ?? stringField(raw.why) ?? stringField(raw.top_reason),
    topSignals: components,
    nextFiles: extractSelectionNextFiles(raw.next ?? raw.next_files, readFirst),
  };
  return Object.values(summary).some((field) => Array.isArray(field) ? field.length > 0 : Boolean(field))
    ? summary
    : undefined;
}

function selectionFileRef(value: unknown, readFirst: ReadFirstFile[]): string | undefined {
  const file = stringField(value);
  if (file) {
    return file;
  }
  if (typeof value === "number" && Number.isInteger(value)) {
    return readFirst[value]?.file;
  }
  return undefined;
}

function selectionEntryScore(entry: unknown[] | undefined, readFirst: ReadFirstFile[]): number | undefined {
  if (!entry) {
    return undefined;
  }
  return numberField(entry[1]) ?? selectionRefScore(entry[0], readFirst);
}

function selectionRefScore(value: unknown, readFirst: ReadFirstFile[]): number | undefined {
  if (typeof value === "number" && Number.isInteger(value)) {
    return readFirst[value]?.score;
  }
  return undefined;
}

function selectionEntryReason(entry: unknown[] | undefined): string | undefined {
  if (!entry) {
    return undefined;
  }
  return stringField(entry[2]) ?? stringField(entry[1]);
}

function selectionSignalName(name: string | undefined): string | undefined {
  switch (name) {
    case "sym":
      return "exact_symbol";
    case "sy":
      return "symbol_name_keyword_cluster";
    case "sub":
      return "symbol_substring";
    case "kw":
      return "keyword_overlap";
    case "p":
      return "path_filename";
    case "pt":
      return "path_keyword_overlap";
    case "mod":
      return "module_anchor";
    case "pi":
      return "path_intent_cluster";
    case "fn":
      return "filename_keyword_cluster";
    case "ct":
      return "content_keyword_overlap";
    case "tf":
      return "test_file";
    case "test":
      return "test_proximity";
    case "cfg":
      return "config_file";
    case "cfgdep":
      return "config_dependency_intent";
    case "dep":
      return "dependency_manifest_intent";
    case "bench":
      return "benchmark_evidence_file_intent";
    case "readme":
      return "readme_evidence_file_intent";
    case "comp":
      return "competitive_positioning_doc";
    case "doc":
      return "docs_intent";
    case "docp":
      return "docs_path_intent";
    case "cmd":
      return "command_surface_intent";
    case "hook":
      return "hook_meta_intent";
    case "im":
      return "graph_imported_file";
    case "ref":
      return "graph_referencing_file";
    case "call":
      return "graph_callee";
    case "caller":
      return "graph_caller";
    case "trace":
      return "stack_trace";
    case "git":
      return "git_signal";
    case "semr":
      return "semantic_recall";
    case "seme":
      return "semantic_embedding";
    default:
      return name;
  }
}

function extractGraphHints(value: unknown): GraphHints | undefined {
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  const hints: GraphHints = {
    upstream: stringArrayField(raw.u ?? raw.upstream),
    downstream: stringArrayField(raw.d ?? raw.downstream),
  };
  return Object.values(hints).some((field) => Array.isArray(field) && field.length > 0)
    ? hints
    : undefined;
}

function extractCallPaths(value: unknown, readFirst: ReadFirstFile[]): CallPaths | undefined {
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  const callPaths: CallPaths = {
    calls: extractCallPathEdges(raw.c ?? raw.calls, readFirst),
    calledBy: extractCallPathEdges(raw.by ?? raw.called_by, readFirst),
  };
  return Object.values(callPaths).some((field) => Array.isArray(field) && field.length > 0)
    ? callPaths
    : undefined;
}

function extractCallPathEdges(value: unknown, readFirst: ReadFirstFile[]): CallPathEdge[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const edges = value
    .map((entry): CallPathEdge | undefined => {
      if (!entry || typeof entry !== "object") {
        return undefined;
      }
      const raw = entry as Record<string, unknown>;
      const file = selectionFileRef(raw.f ?? raw.file, readFirst);
      const to = stringField(raw.t) ?? stringField(raw.to);
      if (!file || !to) {
        return undefined;
      }
      const edge: CallPathEdge = { file, to };
      const from = stringField(raw.fr) ?? stringField(raw.from);
      const line = numberField(raw.l) ?? numberField(raw.line);
      if (from) {
        edge.from = from;
      }
      if (typeof line === "number") {
        edge.line = line;
      }
      return edge;
    })
    .filter((edge): edge is CallPathEdge => Boolean(edge));
  return edges.length ? edges : undefined;
}

function extractSelectionNextFiles(
  value: unknown,
  readFirst: ReadFirstFile[],
): SelectionNextFile[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const files = value
    .map((entry): SelectionNextFile | undefined => {
      if (Array.isArray(entry)) {
        const file = selectionFileRef(entry[0], readFirst);
        if (!file) {
          return undefined;
        }
        const nextFile: SelectionNextFile = { file };
        const score = selectionEntryScore(entry, readFirst);
        const reason = selectionEntryReason(entry);
        if (typeof score === "number") {
          nextFile.score = score;
        }
        if (reason) {
          nextFile.reason = reason;
        }
        return nextFile;
      }
      if (!entry || typeof entry !== "object") {
        return undefined;
      }
      const raw = entry as Record<string, unknown>;
      const file = stringField(raw.f) ?? stringField(raw.file);
      if (!file) {
        return undefined;
      }
      const nextFile: SelectionNextFile = { file };
      const score = numberField(raw.s) ?? numberField(raw.score);
      const reason = stringField(raw.why) ?? stringField(raw.reason);
      if (typeof score === "number") {
        nextFile.score = score;
      }
      if (reason) {
        nextFile.reason = reason;
      }
      return nextFile;
    })
    .filter((entry): entry is SelectionNextFile => Boolean(entry));
  return files.length ? files : undefined;
}

function expansionCommands(
  value: unknown,
  action: "focus" | "related" | "tests" | "grep",
  root: string,
  topFile?: string,
  readFirst: ReadFirstFile[] = [],
): string[] | undefined {
  if (!Array.isArray(value)) {
    const command = expansionCommand(value, action, root, topFile, readFirst);
    return command ? [command] : undefined;
  }
  const commands = value
    .map((entry) => expansionCommand(entry, action, root, topFile, readFirst))
    .filter((command): command is string => Boolean(command));
  return commands.length ? commands : undefined;
}

function expansionCommand(
  value: unknown,
  action: "focus" | "related" | "tests" | "grep",
  root: string,
  topFile?: string,
  readFirst: ReadFirstFile[] = [],
): string | undefined {
  if (typeof value === "string") {
    if ((action === "related" || action === "tests") && !looksLikeCommand(value)) {
      return callsieveFileCommand(action, root, value);
    }
    return value;
  }
  if ((value === true || value === 1) && (action === "related" || action === "tests") && topFile) {
    return callsieveFileCommand(action, root, topFile);
  }
  const compactTarget = expansionFocusTarget(value, readFirst);
  if (compactTarget) {
    return callsieveFocusCommand(
      root,
      compactTarget.file,
      compactTarget.symbol,
      compactTarget.line,
    );
  }
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as {
    tool?: unknown;
    arguments?: { file?: unknown; symbol?: unknown; line?: unknown };
  };
  const tool = stringField(raw.tool);
  if (!tool) {
    return undefined;
  }
  const file = stringField(raw.arguments?.file);
  if (!file) {
    return tool;
  }
  const symbol = stringField(raw.arguments?.symbol);
  const line = numberField(raw.arguments?.line);
  const lineArg = typeof line === "number" ? ` --line ${line}` : "";
  return symbol
    ? `${tool} --file ${file} --symbol ${symbol}${lineArg}`
    : `${tool} --file ${file}${lineArg}`;
}

function expansionTargetFile(value: unknown, readFirst: ReadFirstFile[] = []): string | undefined {
  const compactTarget = expansionFocusTarget(value, readFirst);
  if (compactTarget) {
    return compactTarget.file;
  }
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  const direct = stringField(raw.f) ?? stringField(raw.file);
  if (direct) {
    return direct;
  }
  const args = raw.arguments ?? raw.args;
  if (args && typeof args === "object") {
    return stringField((args as Record<string, unknown>).file);
  }
  return undefined;
}

function expansionFocusTarget(
  value: unknown,
  readFirst: ReadFirstFile[] = [],
): { file: string; symbol?: string; line?: number } | undefined {
  if (typeof value === "number" && Number.isInteger(value)) {
    const selected = readFirst[value];
    return selected ? { file: selected.file, symbol: selected.topSymbol, line: selected.topSymbolLine } : undefined;
  }
  if (Array.isArray(value)) {
    const file = stringField(value[0]);
    if (!file) {
      return undefined;
    }
    return {
      file,
      symbol: stringField(value[1]),
      line: numberField(value[2]),
    };
  }
  if (!value || typeof value !== "object") {
    return undefined;
  }
  const raw = value as Record<string, unknown>;
  if ("tool" in raw) {
    return undefined;
  }
  const file = stringField(raw.f) ?? stringField(raw.file);
  if (!file) {
    return undefined;
  }
  return {
    file,
    symbol: stringField(raw.sy) ?? stringField(raw.symbol),
    line: numberField(raw.l) ?? numberField(raw.line),
  };
}

function callsieveFocusCommand(root: string, file: string, symbol?: string, line?: number): string {
  const symbolArg = symbol ? ` --symbol ${symbol}` : "";
  const lineArg = typeof line === "number" ? ` --line ${line}` : "";
  return `callsieve focus ${root || "."} --file ${file}${symbolArg}${lineArg}`;
}

function callsieveFileCommand(action: "related" | "tests", root: string, file: string): string {
  return `callsieve ${action} ${root || "."} --file ${file}`;
}

function looksLikeCommand(value: string): boolean {
  return value.includes("callsieve ") || value.startsWith("callsieve_");
}

function stringField(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function stringArrayField(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) {
    return undefined;
  }
  const values = value.filter((entry): entry is string => typeof entry === "string" && entry.length > 0);
  return values.length ? values : undefined;
}
