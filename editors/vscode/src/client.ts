import { spawn, ChildProcessWithoutNullStreams, spawnSync } from "child_process";
import * as path from "path";
import * as fs from "fs";
import * as vscode from "vscode";

export interface ReadFirstFile {
  file: string;
  score: number;
  topSymbol?: string;
}

export interface ContextPacket {
  task: string;
  root: string;
  readFirst: ReadFirstFile[];
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
  file?: string;
  score?: number;
  symbols?: Array<{ name?: string }>;
}

function parseStructured(result: McpToolResult, task: string, root: string): ContextPacket {
  if (result.isError) {
    const message = result.content?.find((c) => c.type === "text")?.text ?? "MCP returned error";
    throw new Error(message);
  }
  const structured = result.structuredContent;
  return extractReadFirst(structured, task, root);
}

function parseAgentContextOutput(parsed: unknown, task: string, root: string): ContextPacket {
  // The agent-context CLI wraps the read-first list inside `context`.
  if (parsed && typeof parsed === "object") {
    const ctx = (parsed as { context?: unknown }).context;
    if (ctx) {
      return extractReadFirst(ctx, task, root);
    }
  }
  return extractReadFirst(parsed, task, root);
}

function extractReadFirst(structured: unknown, task: string, root: string): ContextPacket {
  const readFirst: ReadFirstFile[] = [];
  if (structured && typeof structured === "object") {
    const arr = (structured as { read_first?: ReadFirstRaw[] }).read_first;
    if (Array.isArray(arr)) {
      for (const item of arr) {
        if (!item || typeof item.file !== "string") {
          continue;
        }
        const topSymbol = Array.isArray(item.symbols) && item.symbols[0]?.name
          ? item.symbols[0].name
          : undefined;
        readFirst.push({
          file: item.file,
          score: typeof item.score === "number" ? item.score : 0,
          topSymbol,
        });
      }
    }
  }
  return { task, root, readFirst };
}
