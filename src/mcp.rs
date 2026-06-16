use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{indexer, query, store};

pub const FIRST_MILE_MCP_TOOLS: &[&str] = &[
    "callsieve_context",
    "callsieve_focus",
    "callsieve_related",
    "callsieve_tests",
    "callsieve_status",
];
pub const CONTEXT_STRUCTURED_CONTENT_FIELDS: &[&str] = &[
    "read_first",
    "sel",
    "instruction",
    "freshness",
    "retrieval_cost",
    "stats",
    "trace_event",
];
pub const CONTEXT_INSTRUCTION_EXPANSION_KEYS: &[&str] = &["o", "next", "rel", "tests"];
pub const CONTEXT_FRESHNESS_FIELDS: &[&str] = &[
    "initial_fresh",
    "refreshed",
    "final_fresh",
    "index_generation",
    "stale_files",
    "fix_command",
];
pub const CONTEXT_DEFAULT_PROFILE: &str = "skim";
pub const CONTEXT_STRUCTURED_CONTENT_CONTRACT_VERSION: u16 = 1;

/// The MCP server is a long-running process, but parsing a large index.json
/// dominates per-call latency. Cache the parsed index for the last-used root
/// and revalidate freshness (stat-level checks only) before each reuse.
static INDEX_CACHE: Mutex<Option<(PathBuf, Arc<store::CodeIndex>)>> = Mutex::new(None);

fn cached_index(path: &Path) -> Option<Arc<store::CodeIndex>> {
    INDEX_CACHE
        .lock()
        .ok()?
        .as_ref()
        .filter(|(root, _)| root == path)
        .map(|(_, index)| Arc::clone(index))
}

fn remember_index(path: &Path, index: &Arc<store::CodeIndex>) {
    if let Ok(mut guard) = INDEX_CACHE.lock() {
        *guard = Some((path.to_path_buf(), Arc::clone(index)));
    }
}

/// Reuse the in-process index when it is still fresh; otherwise fall back to
/// a disk load (another process may have refreshed index.json) and cache the
/// result. Errors mirror `store::json_store::load_index`.
fn load_index_cached(path: &Path) -> Result<Arc<store::CodeIndex>> {
    if let Some(index) = cached_index(path)
        && query::index_status(path, Some(&index)).is_fresh()
    {
        return Ok(index);
    }
    let index = Arc::new(store::json_store::load_index(path)?);
    remember_index(path, &index);
    Ok(index)
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

pub fn run() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        if let Some(response) = handle_line(&line) {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }
    }

    Ok(())
}

fn handle_line(line: &str) -> Option<Value> {
    match serde_json::from_str::<JsonRpcRequest>(line) {
        Ok(request) => handle_request(request),
        Err(error) => Some(jsonrpc_error(
            Value::Null,
            -32700,
            format!("parse error: {error}"),
        )),
    }
}

fn handle_request(request: JsonRpcRequest) -> Option<Value> {
    let id = request.id?;

    let result = match request.method.as_str() {
        "initialize" => Ok(initialize_result()),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => call_tool(request.params),
        method => Err(anyhow!("unknown method: {method}")),
    };

    Some(match result {
        Ok(result) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        }),
        Err(error) => jsonrpc_error(id, -32602, error.to_string()),
    })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "callsieve",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "callsieve_context",
                "description": "Zero-AI-model-token local retrieval. Preferred first tool for codebase discovery before grep or broad file reads.",
                "annotations": {
                    "title": "CallSieve Context",
                    "readOnlyHint": true
                },
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/index.json."
                        },
                        "task": {
                            "type": "string",
                            "description": "Coding task or natural-language question."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "default": query::DEFAULT_AGENT_CONTEXT_LIMIT
                        },
                        "snippets_per_file": {
                            "type": "integer",
                            "minimum": 0,
                            "default": 0
                        },
                        "no_snippets": {
                            "type": "boolean",
                            "default": false
                        },
                        "profile": {
                            "type": "string",
                            "enum": ["skim", "normal", "full"],
                            "default": "skim"
                        },
                        "token_budget": {
                            "type": "integer",
                            "minimum": 1,
                            "default": 1200
                        }
                    },
                    "required": ["path", "task"]
                }
            },
            {
                "name": "callsieve_symbol",
                "description": "Find indexed symbols by name with imports and reference hints.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/index.json."
                        },
                        "symbol_name": {
                            "type": "string",
                            "description": "Symbol name or substring."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "default": 20
                        }
                    },
                    "required": ["path", "symbol_name"]
                }
            },
            {
                "name": "callsieve_focus",
                "description": "Return targeted symbols, bounded code-unit snippets, compact caller/callee edges, and related tests for one indexed file selected by CallSieve. Pass symbol and line to inspect the exact selected code unit before reading the whole file. Set references true only when non-call reference edges are needed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/index.json."
                        },
                        "file": {
                            "type": "string",
                            "description": "Indexed file path returned by callsieve_context."
                        },
                        "symbol": {
                            "type": "string",
                            "description": "Optional symbol name from read_first[].sy[0][0] for skim packets, or from symbols[].name in normal/full packets."
                        },
                        "line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Optional 1-based line from read_first[].sy[0][1] for skim packets, or symbol line fields in normal/full packets."
                        },
                        "references": {
                            "type": "boolean",
                            "default": false,
                            "description": "Include non-call reference edges. Omitted by default to keep focus packets compact."
                        },
                        "snippets_per_symbol": {
                            "type": "integer",
                            "minimum": 0,
                            "default": 1
                        },
                        "skeleton": {
                            "type": "boolean",
                            "default": false,
                            "description": "Collapse symbol bodies to signature + `{ … }` markers for a compact, low-token view of the file's shape."
                        }
                    },
                    "required": ["path", "file"]
                }
            },
            {
                "name": "callsieve_related",
                "description": "Return import, caller, callee, and blast-radius hints for one indexed file.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "file": {"type": "string"}
                    },
                    "required": ["path", "file"]
                }
            },
            {
                "name": "callsieve_tests",
                "description": "Return tests likely related to one indexed file.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "file": {"type": "string"}
                    },
                    "required": ["path", "file"]
                }
            },
            {
                "name": "callsieve_stats",
                "description": "Show compact index statistics for a repository.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/index.json."
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "callsieve_status",
                "description": "Show index freshness, watch, schema, and LSP-enrichment status for a repository.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/index.json."
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "callsieve_trace_check",
                "description": "Check whether an observed agent trace used grep before callsieve_context.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "trace_json": {
                            "type": "string",
                            "description": "Trace JSON using the benchmark session shape."
                        },
                        "strict": {
                            "type": "boolean",
                            "default": false,
                            "description": "Also fail file reads before callsieve_context."
                        }
                    },
                    "required": ["trace_json"]
                }
            },
            {
                "name": "callsieve_benchmark",
                "description": "Estimate grep/read-loop token savings for a coding task.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/index.json."
                        },
                        "task": {
                            "type": "string",
                            "description": "Coding task or natural-language question."
                        },
                        "limit": {
                            "type": "integer",
                            "minimum": 1,
                            "default": 8
                        },
                        "snippets_per_file": {
                            "type": "integer",
                            "minimum": 0,
                            "default": 2
                        },
                        "no_snippets": {
                            "type": "boolean",
                            "default": false
                        },
                        "profile": {
                            "type": "string",
                            "enum": ["skim", "normal", "full"],
                            "default": "normal"
                        },
                        "token_budget": {
                            "type": "integer",
                            "minimum": 1
                        }
                    },
                    "required": ["path", "task"]
                }
            },
            {
                "name": "callsieve_memory_recall",
                "description": "Recall similar past tasks and their read-first files from local task memory (Agent Memory Protocol verb: amp.recall). Read-only; does not write the current task.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/."
                        },
                        "task": {
                            "type": "string",
                            "description": "Coding task or natural-language question to match against remembered tasks."
                        }
                    },
                    "required": ["path", "task"]
                }
            },
            {
                "name": "callsieve_memory_stats",
                "description": "Summary metrics for this repo's task memory: entry count, contributing clients, last task (Agent Memory Protocol verb: amp.stats).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/."
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "callsieve_memory_export",
                "description": "Export this repo's task memory for portability (Agent Memory Protocol verb: amp.export). Use the vendor-neutral Memory Exchange Format (mxf) to round-trip with other agent-memory tools.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/."
                        },
                        "format": {
                            "type": "string",
                            "enum": ["mxf", "json"],
                            "default": "mxf",
                            "description": "mxf = vendor-neutral Memory Exchange Format; json = CallSieve native store shape."
                        }
                    },
                    "required": ["path"]
                }
            },
            {
                "name": "callsieve_memory_import",
                "description": "Merge an exported memory document into this repo's task memory (Agent Memory Protocol verb: amp.import). Accepts the vendor-neutral Memory Exchange Format (mxf) or CallSieve native json.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/."
                        },
                        "document": {
                            "type": "string",
                            "description": "Serialized memory document to import."
                        },
                        "format": {
                            "type": "string",
                            "enum": ["mxf", "json"],
                            "default": "mxf",
                            "description": "Format of the supplied document."
                        }
                    },
                    "required": ["path", "document"]
                }
            },
            {
                "name": "callsieve_memory_forget",
                "description": "Clear this repo's local task memory (Agent Memory Protocol verb: amp.forget). Coarse: forgets all remembered tasks for the repo.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Repository root containing .callsieve/."
                        }
                    },
                    "required": ["path"]
                }
            }
        ]
    })
}

pub fn listed_tool_names() -> Vec<String> {
    tools_list_result()
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

fn call_tool(params: Option<Value>) -> Result<Value> {
    let params = params.ok_or_else(|| anyhow!("tools/call requires params"))?;
    let name = required_str(&params, "name")?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "callsieve_context" => Ok(execute_context_tool(&arguments)),
        "callsieve_symbol" => Ok(tool_execution_result(execute_symbol(&arguments))),
        "callsieve_focus" => Ok(tool_execution_result(execute_focus(&arguments))),
        "callsieve_related" => Ok(tool_execution_result(execute_related(&arguments))),
        "callsieve_tests" => Ok(tool_execution_result(execute_tests(&arguments))),
        "callsieve_stats" => Ok(tool_execution_result(execute_stats(&arguments))),
        "callsieve_status" => Ok(tool_execution_result(execute_status(&arguments))),
        "callsieve_trace_check" => Ok(tool_execution_result(execute_trace_check(&arguments))),
        "callsieve_benchmark" => Ok(tool_execution_result(execute_benchmark(&arguments))),
        "callsieve_memory_recall" => Ok(memory_tool_result(execute_memory_recall(&arguments))),
        "callsieve_memory_stats" => Ok(memory_tool_result(execute_memory_stats(&arguments))),
        "callsieve_memory_export" => Ok(memory_tool_result(execute_memory_export(&arguments))),
        "callsieve_memory_import" => Ok(memory_tool_result(execute_memory_import(&arguments))),
        "callsieve_memory_forget" => Ok(memory_tool_result(execute_memory_forget(&arguments))),
        name => Err(anyhow!("unknown tool: {name}")),
    }
}

fn execute_context_tool(arguments: &Value) -> Value {
    match execute_context(arguments) {
        Ok(value) => tool_execution_result(Ok(value)),
        Err(error) => {
            let fix_command = arguments
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .map(|path| format!("callsieve index {path}"));
            tool_execution_error(error.to_string(), fix_command)
        }
    }
}

fn execute_context(arguments: &Value) -> Result<Value> {
    let total_start = Instant::now();
    let path = repo_path(arguments)?;
    let task = required_str(arguments, "task")?;
    let limit = optional_usize(arguments, "limit", query::DEFAULT_AGENT_CONTEXT_LIMIT)?;
    let snippets_per_file = optional_usize(arguments, "snippets_per_file", 0)?;
    let include_snippets = !optional_bool(arguments, "no_snippets", false)?;
    let profile = optional_context_profile(arguments, "profile", query::ContextProfile::Skim)?;
    let token_budget = optional_usize(
        arguments,
        "token_budget",
        query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET,
    )?;
    let freshness_start = Instant::now();
    // Prefer the in-process cache when fresh; a stale cache falls back to a
    // disk load because another process may have refreshed index.json.
    let initial_index = cached_index(&path)
        .filter(|index| query::index_status(&path, Some(index)).is_fresh())
        .or_else(|| store::json_store::load_index(&path).ok().map(Arc::new));
    let initial_status = query::index_status(&path, initial_index.as_deref());
    let initial_status_value = serde_json::to_value(&initial_status)?;
    let initial_fresh = status_is_fresh(&initial_status_value);
    let freshness_check_ms = elapsed_ms(freshness_start.elapsed());
    let mut refreshed = false;
    let mut rebuild_ms = 0;
    let index = if initial_fresh {
        initial_index.expect("fresh status requires an index")
    } else {
        let rebuild_start = Instant::now();
        let index = indexer::build_index(&path).map_err(|error| {
            anyhow!(
                "failed to rebuild missing or stale CallSieve index for {}; run `callsieve index {}`: {error}",
                path.display(),
                path.display()
            )
        })?;
        store::json_store::save_index(&path, &index).map_err(|error| {
            anyhow!(
                "failed to save rebuilt CallSieve index for {}; run `callsieve index {}`: {error}",
                path.display(),
                path.display()
            )
        })?;
        refreshed = true;
        rebuild_ms = elapsed_ms(rebuild_start.elapsed());
        Arc::new(index)
    };
    remember_index(&path, &index);
    let final_status = query::index_status(&path, Some(&index));
    let final_status_value = serde_json::to_value(&final_status)?;
    let final_fresh = status_is_fresh(&final_status_value);
    let output = query::build_context(
        &path,
        &index,
        task,
        limit,
        snippets_per_file,
        include_snippets,
    )?;

    let mut value = query::context_value(
        &output,
        query::ContextViewOptions {
            profile,
            token_budget: Some(token_budget),
            include_git: false,
            include_call_paths: false,
        },
    )?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "instruction".to_string(),
            mcp_context_instruction(&path, &output),
        );
        object.insert(
            "freshness".to_string(),
            json!({
                "initial_fresh": initial_fresh,
                "refreshed": refreshed,
                "final_fresh": final_fresh,
                "index_generation": final_status_value
                    .get("index_generation")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                "stale_files": final_status_value
                    .get("stale_files")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                "fix_command": format!("callsieve index {}", path.display())
            }),
        );
        if let Some(timing) = object.get_mut("timing").and_then(Value::as_object_mut) {
            timing.insert(
                "freshness_check_ms".to_string(),
                serde_json::Value::from(freshness_check_ms),
            );
            timing.insert(
                "index_rebuild_ms".to_string(),
                serde_json::Value::from(rebuild_ms),
            );
            timing.insert(
                "mcp_total_ms".to_string(),
                serde_json::Value::from(elapsed_ms(total_start.elapsed())),
            );
        }
        object.insert(
            "trace_event".to_string(),
            json!({
                "tool": "callsieve_context",
                "policy": "first_codebase_discovery_tool",
                "called_at": now_unix_seconds()
            }),
        );
    }
    apply_mcp_context_envelope_budget(&mut value, Some(token_budget))?;

    Ok(value)
}

fn apply_mcp_context_envelope_budget(value: &mut Value, token_budget: Option<usize>) -> Result<()> {
    let Some(token_budget) = token_budget else {
        return Ok(());
    };

    if query::value_estimated_tokens(value)? > token_budget {
        trim_mcp_expansion_fields(value, &["inspect_next_files"]);
    }
    if query::value_estimated_tokens(value)? > token_budget {
        trim_mcp_expansion_fields(
            value,
            &["grep_fallback", "expand_relationships", "inspect_tests"],
        );
    }
    if query::value_estimated_tokens(value)? > token_budget {
        trim_mcp_graph_hints(value);
        refresh_mcp_context_stats(value)?;
    }
    while query::value_estimated_tokens(value)? > token_budget {
        let dropped = {
            let Some(files) = value.get_mut("read_first").and_then(Value::as_array_mut) else {
                return Ok(());
            };
            if files.len() <= 1 {
                false
            } else {
                files.pop();
                true
            }
        };
        if !dropped {
            break;
        }
        refresh_mcp_context_stats(value)?;
    }
    if query::value_estimated_tokens(value)? > token_budget {
        trim_mcp_expansion_fields(value, &["inspect_top_file"]);
    }
    if query::value_estimated_tokens(value)? > token_budget {
        trim_mcp_retrieval_note(value)?;
    }

    Ok(())
}

fn trim_mcp_expansion_fields(value: &mut Value, fields: &[&str]) {
    if let Some(expansion) = value
        .get_mut("instruction")
        .and_then(instruction_expansion_mut)
        .and_then(Value::as_object_mut)
    {
        for field in fields {
            for alias in expansion_field_aliases(field) {
                expansion.remove(*alias);
            }
        }
    }
}

fn instruction_expansion(instruction: &Value) -> Option<&Value> {
    instruction
        .get("x")
        .or_else(|| instruction.get("local_first_expansion"))
}

fn instruction_expansion_mut(instruction: &mut Value) -> Option<&mut Value> {
    if instruction.get("x").is_some() {
        instruction.get_mut("x")
    } else {
        instruction.get_mut("local_first_expansion")
    }
}

fn expansion_field_aliases(field: &str) -> &'static [&'static str] {
    match field {
        "inspect_top_file" | "top" | "o" => &["inspect_top_file", "top", "o"],
        "inspect_next_files" | "next" | "n" => &["inspect_next_files", "next", "n"],
        "expand_relationships" | "rel" | "r" => &["expand_relationships", "rel", "r"],
        "inspect_tests" | "tests" | "t" => &["inspect_tests", "tests", "t"],
        "grep_fallback" | "grep" => &["grep_fallback", "grep"],
        _ => &[],
    }
}

fn trim_mcp_graph_hints(value: &mut Value) {
    let Some(files) = value.get_mut("read_first").and_then(Value::as_array_mut) else {
        return;
    };
    for file in files {
        if let Some(file) = file.as_object_mut() {
            file.remove("g");
            file.remove("cp");
            file.remove("graph_hints");
            file.remove("call_paths");
        }
    }
}

fn refresh_mcp_context_stats(value: &mut Value) -> Result<()> {
    query::trim_selection_summary_to_read_first(value);
    let (selected_files, selected_symbols, related_tests) = value
        .get("read_first")
        .and_then(Value::as_array)
        .map(|files| {
            let selected_symbols = files
                .iter()
                .filter_map(|file| {
                    file.get("sy")
                        .or_else(|| file.get("symbols"))
                        .and_then(Value::as_array)
                })
                .map(Vec::len)
                .sum::<usize>();
            let related_tests = files
                .iter()
                .map(|file| {
                    file.get("related_tests")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .or_else(|| {
                            file.get("i")
                                .or_else(|| file.get("impact"))
                                .and_then(compact_or_legacy_impact_tests_len)
                        })
                        .unwrap_or_default()
                })
                .sum::<usize>();
            (files.len(), selected_symbols, related_tests)
        })
        .unwrap_or_default();

    if let Some(stats) = value.get_mut("stats").and_then(Value::as_object_mut) {
        if stats.contains_key("tokens") || stats.contains_key("local") {
            stats.remove("tokens");
            stats.remove("t");
        } else {
            stats.insert("selected_files".to_string(), json!(selected_files));
            stats.insert("selected_symbols".to_string(), json!(selected_symbols));
            stats.insert("related_tests".to_string(), json!(related_tests));
            stats.remove("estimated_tokens");
        }
    }
    let estimated_tokens = query::value_estimated_tokens(value)?;
    if let Some(stats) = value.get_mut("stats").and_then(Value::as_object_mut) {
        if stats.contains_key("local") {
            stats.insert("t".to_string(), json!(estimated_tokens));
        } else {
            stats.insert("estimated_tokens".to_string(), json!(estimated_tokens));
        }
    }
    Ok(())
}

fn compact_or_legacy_impact_tests_len(impact: &Value) -> Option<usize> {
    if let Some(tests) = impact.get("t").or_else(|| impact.get("tests")) {
        return tests
            .as_array()
            .map(Vec::len)
            .or_else(|| tests.as_str().map(|_| 1));
    }
    let items = impact.as_array()?;
    let test_value = items.get(1)?;
    if test_value.is_string() {
        Some(1)
    } else {
        test_value.as_array().map(Vec::len)
    }
}

fn trim_mcp_retrieval_note(value: &mut Value) -> Result<()> {
    if let Some(retrieval_cost) = value
        .get_mut("retrieval_cost")
        .and_then(Value::as_object_mut)
    {
        retrieval_cost.remove("note");
        refresh_mcp_context_stats(value)?;
    }
    Ok(())
}

fn mcp_context_instruction(path: &Path, context: &query::ContextOutput) -> Value {
    let path = path.display().to_string();
    let targets = query::context_read_first_targets(context);
    let top_target = targets.first();
    let tool_call_for_file = |tool: &str, file: &str| {
        json!({
            "tool": tool,
            "arguments": {
                "path": path.clone(),
                "file": file
            }
        })
    };
    let focus_tool_call_for_target = |target: &query::FocusTarget| {
        let mut arguments = json!({
            "path": path.clone(),
            "file": target.file.clone()
        });
        if let Some(arguments) = arguments.as_object_mut() {
            if let Some(symbol) = target.symbol.as_deref() {
                arguments.insert("symbol".to_string(), json!(symbol));
            }
            if let Some(line) = target.line {
                arguments.insert("line".to_string(), json!(line));
            }
        }
        json!({
            "tool": "callsieve_focus",
            "arguments": arguments
        })
    };
    let tool_call_for_top_code_file = |tool: &str| {
        top_target
            .filter(|target| target.is_code)
            .map(|target| tool_call_for_file(tool, &target.file))
    };
    let inspect_next_files = targets
        .iter()
        .skip(1)
        .take(1)
        .map(focus_tool_call_for_target)
        .collect::<Vec<_>>();

    let mut expansion = serde_json::Map::new();
    if let Some(top_target) = top_target {
        expansion.insert("o".to_string(), focus_tool_call_for_target(top_target));
    }
    if !inspect_next_files.is_empty() {
        expansion.insert("next".to_string(), json!(inspect_next_files));
    }
    if let Some(expand_relationships) = tool_call_for_top_code_file("callsieve_related") {
        expansion.insert("rel".to_string(), expand_relationships);
    }
    if let Some(inspect_tests) = tool_call_for_top_code_file("callsieve_tests") {
        expansion.insert("tests".to_string(), inspect_tests);
    }
    json!({
        "x": expansion
    })
}

fn execute_symbol(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let symbol_name = required_str(arguments, "symbol_name")?;
    let limit = optional_usize(arguments, "limit", 20)?;
    let index = load_index_cached(&path)?;
    let output = query::find_symbol(&path, &index, symbol_name, limit)?;

    Ok(serde_json::to_value(output)?)
}

fn execute_focus(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let file = required_str(arguments, "file")?;
    let symbol = arguments.get("symbol").and_then(Value::as_str);
    let line = optional_usize_opt(arguments, "line")?;
    let include_references = optional_bool(arguments, "references", false)?;
    let snippets_per_symbol = optional_usize(arguments, "snippets_per_symbol", 1)?;
    let skeleton = optional_bool(arguments, "skeleton", false)?;
    let index = load_index_cached(&path)?;
    let output = query::focus_file(
        &path,
        &index,
        file,
        symbol,
        line,
        include_references,
        snippets_per_symbol,
        skeleton,
    )?;

    Ok(serde_json::to_value(output)?)
}

fn execute_related(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let file = required_str(arguments, "file")?;
    let index = load_index_cached(&path)?;
    let output = query::related_file(&path, &index, file)?;

    Ok(serde_json::to_value(output)?)
}

fn execute_tests(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let file = required_str(arguments, "file")?;
    let index = load_index_cached(&path)?;
    let output = query::tests_for_file(&path, &index, file)?;

    Ok(serde_json::to_value(output)?)
}

fn execute_stats(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let index = load_index_cached(&path)?;
    let output = query::stats(&path, &index)?;

    Ok(serde_json::to_value(output)?)
}

fn execute_status(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let index = store::json_store::load_index(&path).ok();
    let output = query::index_status(&path, index.as_ref());

    Ok(serde_json::to_value(output)?)
}

fn execute_trace_check(arguments: &Value) -> Result<Value> {
    let trace_json = required_str(arguments, "trace_json")?;
    let strict = optional_bool(arguments, "strict", false)?;
    let output = if strict {
        query::trace_check_from_str_with_options(trace_json, true)?
    } else {
        query::trace_check_from_str(trace_json)?
    };

    Ok(serde_json::to_value(output)?)
}

fn execute_benchmark(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let task = required_str(arguments, "task")?;
    let limit = optional_usize(arguments, "limit", 8)?;
    let snippets_per_file = optional_usize(arguments, "snippets_per_file", 2)?;
    let include_snippets = !optional_bool(arguments, "no_snippets", false)?;
    let profile = optional_context_profile(arguments, "profile", query::ContextProfile::Normal)?;
    let token_budget = optional_usize_opt(arguments, "token_budget")?;
    let index = load_index_cached(&path)?;
    let output = query::benchmark_context_with_options(
        &path,
        &index,
        task,
        limit,
        snippets_per_file,
        include_snippets,
        query::ContextViewOptions {
            profile,
            token_budget,
            include_git: false,
            include_call_paths: false,
        },
    )?;

    Ok(serde_json::to_value(output)?)
}

/// Error carrying an Agent Memory Protocol error code so memory-verb failures
/// map to a stable contract (`AMP_INVALID_ARGUMENT`, `AMP_INTERNAL`, ...)
/// instead of opaque strings.
#[derive(Debug)]
struct AmpError {
    code: &'static str,
    message: String,
}

impl std::fmt::Display for AmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AmpError {}

fn amp_invalid_argument(error: impl std::fmt::Display) -> anyhow::Error {
    AmpError {
        code: "AMP_INVALID_ARGUMENT",
        message: error.to_string(),
    }
    .into()
}

fn amp_repo_path(arguments: &Value) -> Result<PathBuf> {
    repo_path(arguments).map_err(amp_invalid_argument)
}

fn execute_memory_recall(arguments: &Value) -> Result<Value> {
    let path = amp_repo_path(arguments)?;
    let task = required_str(arguments, "task").map_err(amp_invalid_argument)?;
    Ok(serde_json::to_value(query::recall_task_memory(&path, task)?)?)
}

fn execute_memory_stats(arguments: &Value) -> Result<Value> {
    let path = amp_repo_path(arguments)?;
    Ok(query::task_memory_stats(&path))
}

fn execute_memory_export(arguments: &Value) -> Result<Value> {
    let path = amp_repo_path(arguments)?;
    let format = memory_format_arg(arguments)?;
    let (serialized, entries) = match format {
        MemoryDocumentFormat::Mxf => query::export_task_memory_mxf(&path)?,
        MemoryDocumentFormat::Json => query::export_task_memory(&path)?,
    };
    Ok(json!({
        "format": format.as_str(),
        "entries": entries,
        "document": serialized,
    }))
}

fn execute_memory_import(arguments: &Value) -> Result<Value> {
    let path = amp_repo_path(arguments)?;
    let document = required_str(arguments, "document").map_err(amp_invalid_argument)?;
    let format = memory_format_arg(arguments)?;
    let (imported, total) = match format {
        MemoryDocumentFormat::Mxf => query::merge_task_memory_mxf(&path, document),
        MemoryDocumentFormat::Json => query::merge_task_memory(&path, document),
    }
    .map_err(amp_invalid_argument)?;
    Ok(json!({
        "format": format.as_str(),
        "imported": imported,
        "entries_total": total,
    }))
}

fn execute_memory_forget(arguments: &Value) -> Result<Value> {
    let path = amp_repo_path(arguments)?;
    let cleared = query::clear_task_memory(&path)?;
    Ok(json!({ "cleared": cleared }))
}

#[derive(Clone, Copy)]
enum MemoryDocumentFormat {
    Mxf,
    Json,
}

impl MemoryDocumentFormat {
    fn as_str(self) -> &'static str {
        match self {
            MemoryDocumentFormat::Mxf => "mxf",
            MemoryDocumentFormat::Json => "json",
        }
    }
}

/// Parses the optional `format` argument for memory verbs. Defaults to the
/// portable MXF format, matching the tool schema.
fn memory_format_arg(arguments: &Value) -> Result<MemoryDocumentFormat> {
    match arguments.get("format").and_then(Value::as_str) {
        None | Some("mxf") => Ok(MemoryDocumentFormat::Mxf),
        Some("json") => Ok(MemoryDocumentFormat::Json),
        Some(other) => Err(amp_invalid_argument(format!(
            "unknown format '{other}' (expected mxf or json)"
        ))),
    }
}

/// Wraps a memory-verb result in the MCP tool envelope, mapping failures to an
/// Agent Memory Protocol error code (carried by `AmpError`, else `AMP_INTERNAL`).
fn memory_tool_result(result: Result<Value>) -> Value {
    match result {
        Ok(value) => tool_execution_result(Ok(value)),
        Err(error) => {
            let code = error
                .downcast_ref::<AmpError>()
                .map_or("AMP_INTERNAL", |amp| amp.code);
            amp_error_value(code, error.to_string())
        }
    }
}

fn amp_error_value(code: &str, message: String) -> Value {
    let structured_content = json!({
        "error": {
            "code": code,
            "message": message
        }
    });
    let text = serde_json::to_string_pretty(&structured_content)
        .unwrap_or_else(|_| structured_content.to_string());
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "structuredContent": structured_content,
        "isError": true
    })
}

fn repo_path(arguments: &Value) -> Result<PathBuf> {
    Ok(PathBuf::from(required_str(arguments, "path")?))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| anyhow!("missing string field: {field}"))
}

fn optional_bool(value: &Value, field: &str, default: bool) -> Result<bool> {
    match value.get(field) {
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow!("field must be boolean: {field}")),
        None => Ok(default),
    }
}

fn optional_context_profile(
    value: &Value,
    field: &str,
    default: query::ContextProfile,
) -> Result<query::ContextProfile> {
    match value.get(field) {
        Some(value) => match value
            .as_str()
            .ok_or_else(|| anyhow!("field must be string: {field}"))?
        {
            "skim" => Ok(query::ContextProfile::Skim),
            "normal" => Ok(query::ContextProfile::Normal),
            "full" => Ok(query::ContextProfile::Full),
            other => Err(anyhow!("unsupported context profile: {other}")),
        },
        None => Ok(default),
    }
}

fn optional_usize(value: &Value, field: &str, default: usize) -> Result<usize> {
    match value.get(field) {
        Some(value) => {
            let number = value
                .as_u64()
                .ok_or_else(|| anyhow!("field must be a non-negative integer: {field}"))?;
            usize::try_from(number).map_err(|_| anyhow!("field is too large: {field}"))
        }
        None => Ok(default),
    }
}

fn optional_usize_opt(value: &Value, field: &str) -> Result<Option<usize>> {
    match value.get(field) {
        Some(value) => {
            let number = value
                .as_u64()
                .ok_or_else(|| anyhow!("field must be a non-negative integer: {field}"))?;
            Ok(Some(
                usize::try_from(number).map_err(|_| anyhow!("field is too large: {field}"))?,
            ))
        }
        None => Ok(None),
    }
}

fn status_is_fresh(status: &Value) -> bool {
    status
        .get("fresh")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn elapsed_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn tool_execution_result(result: Result<Value>) -> Value {
    match result {
        Ok(structured_content) => {
            let text = tool_text_summary(&structured_content);
            json!({
                "content": [
                    {
                        "type": "text",
                        "text": text
                    }
                ],
                "structuredContent": structured_content,
                "isError": false
            })
        }
        Err(error) => json!({
            "content": [
                {
                    "type": "text",
                    "text": error.to_string()
                }
            ],
            "isError": true
        }),
    }
}

fn tool_text_summary(value: &Value) -> String {
    if let Some(files) = value.get("read_first").and_then(Value::as_array) {
        let names = files
            .iter()
            .filter_map(|file| {
                file.get("f")
                    .or_else(|| file.get("file"))
                    .and_then(Value::as_str)
            })
            .take(3)
            .collect::<Vec<_>>();
        let count = files.len();
        let packet = packet_token_summary(value);
        let next = next_local_tools_summary(value);
        if names.is_empty() {
            return format!(
                "CallSieve used zero retrieval-model tokens{packet}; selected no read-first files. {next}Details in structuredContent."
            );
        }
        return format!(
            "CallSieve used zero retrieval-model tokens{packet}; selected {count} read-first files: {}. {}{next}Details in structuredContent.",
            names.join(", "),
            selection_text_summary(value),
        );
    }

    if let Some(matches) = value.get("matches").and_then(Value::as_array) {
        return format!(
            "CallSieve returned {} symbol/query matches. See structuredContent for details.",
            matches.len()
        );
    }

    if let Some(status) = value.get("status").and_then(Value::as_str) {
        return format!("CallSieve status: {status}. See structuredContent for details.");
    }

    "CallSieve tool result is available in structuredContent.".to_string()
}

fn packet_token_summary(value: &Value) -> String {
    let Some(stats) = value.get("stats") else {
        return String::new();
    };
    let Some(estimated) = stats
        .get("t")
        .or_else(|| stats.get("tokens"))
        .or_else(|| stats.get("estimated_tokens"))
        .and_then(Value::as_u64)
    else {
        return String::new();
    };
    match stats
        .get("b")
        .or_else(|| stats.get("budget"))
        .or_else(|| stats.get("token_budget"))
        .and_then(Value::as_u64)
    {
        Some(budget) => format!("; packet {estimated}/{budget} est. tokens"),
        None => format!("; packet {estimated} est. tokens"),
    }
}

fn next_local_tools_summary(value: &Value) -> String {
    let Some(expansion) = value.get("instruction").and_then(instruction_expansion) else {
        return String::new();
    };
    let tools = ["inspect_top_file", "expand_relationships", "inspect_tests"]
        .into_iter()
        .filter_map(|field| {
            expansion_field_value(expansion, field)
                .and_then(|entry| entry.get("tool"))
                .and_then(Value::as_str)
                .map(|tool| {
                    if field == "inspect_top_file"
                        && entry_has_symbol_argument(expansion_field_value(expansion, field))
                    {
                        format!("{tool} (symbol-scoped)")
                    } else {
                        tool.to_string()
                    }
                })
        })
        .collect::<Vec<_>>();
    if tools.is_empty() {
        String::new()
    } else {
        format!("Use {} before grep. ", tools.join(", "))
    }
}

fn expansion_field_value<'a>(expansion: &'a Value, field: &str) -> Option<&'a Value> {
    expansion_field_aliases(field)
        .iter()
        .find_map(|alias| expansion.get(*alias))
}

fn entry_has_symbol_argument(entry: Option<&Value>) -> bool {
    entry
        .and_then(|entry| entry.get("arguments"))
        .and_then(|arguments| arguments.get("symbol"))
        .and_then(Value::as_str)
        .is_some()
}

fn selection_text_summary(value: &Value) -> String {
    let Some(selection) = value.get("sel").or_else(|| value.get("selection_summary")) else {
        return String::new();
    };
    let Some(component) = selection
        .get("sig")
        .or_else(|| selection.get("top_signals"))
        .and_then(Value::as_array)
        .and_then(|components| components.first())
    else {
        return String::new();
    };
    let Some(name) = selection_signal_name(component) else {
        return String::new();
    };
    format!("Top local signal: {name}. ")
}

fn selection_signal_name(component: &Value) -> Option<&str> {
    let name = component.as_str().or_else(|| {
        component
            .get("n")
            .or_else(|| component.get("name"))
            .and_then(Value::as_str)
            .or_else(|| {
                component
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(Value::as_str)
            })
    })?;
    Some(expand_compact_selection_signal(name))
}

fn expand_compact_selection_signal(name: &str) -> &str {
    match name {
        "sym" => "exact_symbol",
        "sy" => "symbol_name_keyword_cluster",
        "sub" => "symbol_substring",
        "kw" => "keyword_overlap",
        "p" => "path_filename",
        "pt" => "path_keyword_overlap",
        "mod" => "module_anchor",
        "pi" => "path_intent_cluster",
        "fn" => "filename_keyword_cluster",
        "ct" => "content_keyword_overlap",
        "tf" => "test_file",
        "test" => "test_proximity",
        "cfg" => "config_file",
        "cfgdep" => "config_dependency_intent",
        "dep" => "dependency_manifest_intent",
        "bench" => "benchmark_evidence_file_intent",
        "readme" => "readme_evidence_file_intent",
        "comp" => "competitive_positioning_doc",
        "doc" => "docs_intent",
        "docp" => "docs_path_intent",
        "cmd" => "command_surface_intent",
        "hook" => "hook_meta_intent",
        "im" => "graph_imported_file",
        "ref" => "graph_referencing_file",
        "call" => "graph_callee",
        "caller" => "graph_caller",
        "trace" => "stack_trace",
        "git" => "git_signal",
        "semr" => "semantic_recall",
        "seme" => "semantic_embedding",
        _ => name,
    }
}

fn tool_execution_error(message: String, fix_command: Option<String>) -> Value {
    let structured_content = json!({
        "error": {
            "message": message,
            "fix_command": fix_command
        }
    });
    let text = serde_json::to_string_pretty(&structured_content)
        .unwrap_or_else(|_| structured_content.to_string());
    json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ],
        "structuredContent": structured_content,
        "isError": true
    })
}

fn jsonrpc_error(id: Value, code: i32, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer;
    use std::fs;

    /// INDEX_CACHE is process-global with a single slot; tests that exercise
    /// index-bearing handlers must not interleave or they evict each other's
    /// entries mid-assertion (a CI-only flake at 2-core parallelism).
    static CACHE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn cache_lock() -> std::sync::MutexGuard<'static, ()> {
        CACHE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(path: impl AsRef<std::path::Path>, content: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn lists_callsieve_tools() {
        let response = handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();

        assert!(tools.iter().any(|tool| tool["name"] == "callsieve_context"));
        let context = tools
            .iter()
            .find(|tool| tool["name"] == "callsieve_context")
            .expect("callsieve_context should be listed");
        assert_eq!(
            context["inputSchema"]["properties"]["limit"]["default"],
            query::DEFAULT_AGENT_CONTEXT_LIMIT
        );
        let focus = tools
            .iter()
            .find(|tool| tool["name"] == "callsieve_focus")
            .expect("callsieve_focus should be listed");
        assert!(
            focus["description"]
                .as_str()
                .unwrap()
                .contains("exact selected code unit")
        );
        assert!(
            focus["inputSchema"]["properties"]["symbol"]["description"]
                .as_str()
                .unwrap()
                .contains("sy[0][0]")
        );
        assert!(
            focus["inputSchema"]["properties"]["line"]["description"]
                .as_str()
                .unwrap()
                .contains("sy[0][1]")
        );
    }

    #[test]
    fn listed_tool_names_cover_first_mile_mcp_tools() {
        let names = listed_tool_names();

        for required in FIRST_MILE_MCP_TOOLS {
            assert!(
                names.iter().any(|name| name == required),
                "missing first-mile MCP tool {required}"
            );
        }
    }

    #[test]
    fn context_tool_returns_structured_content() {
        let _guard = cache_lock();
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("src/auth/session.ts"),
            "export function createSession(userId: string) {\n  return `token:${userId}`;\n}\n",
        );
        let index = indexer::build_index(temp.path()).unwrap();
        store::json_store::save_index(temp.path(), &index).unwrap();
        let path = temp.path().to_string_lossy().replace('\\', "\\\\");
        let request = format!(
            r#"{{
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {{
                    "name": "callsieve_context",
                    "arguments": {{
                        "path": "{path}",
                        "task": "change createSession token behavior",
                        "limit": 3
                    }}
                }}
            }}"#
        );

        let response = handle_line(&request).unwrap();

        assert_eq!(response["result"]["isError"], false);
        for field in CONTEXT_STRUCTURED_CONTENT_FIELDS {
            assert!(
                response["result"]["structuredContent"]
                    .get(*field)
                    .is_some(),
                "missing MCP structuredContent contract field {field}"
            );
        }
        assert_eq!(
            response["result"]["structuredContent"]["read_first"][0]["f"],
            "src/auth/session.ts"
        );
        assert_eq!(
            response["result"]["structuredContent"]["instruction"]["x"]["o"]["tool"],
            "callsieve_focus"
        );
        assert!(
            response["result"]["structuredContent"]["instruction"]["x"]
                .get("top")
                .is_none()
        );
        assert!(
            query::value_estimated_tokens(&response["result"]["structuredContent"]).unwrap()
                <= query::DEFAULT_AGENT_CONTEXT_TOKEN_BUDGET
        );
    }

    #[test]
    fn context_tool_omits_code_followups_for_docs_top_file() {
        let _guard = cache_lock();
        let temp = tempfile::tempdir().unwrap();
        write(
            temp.path().join("docs/COMPETITIVE.md"),
            "# Competitive Notes\n\nCallSieve should beat Cursor and Copilot with local token-saving retrieval before agents spend context.\n",
        );
        write(
            temp.path().join("src/cli.rs"),
            "pub fn agent_context() {}\n",
        );
        let index = indexer::build_index(temp.path()).unwrap();
        store::json_store::save_index(temp.path(), &index).unwrap();
        let path = temp.path().to_string_lossy().replace('\\', "\\\\");
        let request = format!(
            r#"{{
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {{
                    "name": "callsieve_context",
                    "arguments": {{
                        "path": "{path}",
                        "task": "competitive local token savings context",
                        "limit": 3
                    }}
                }}
            }}"#
        );

        let response = handle_line(&request).unwrap();
        let structured = &response["result"]["structuredContent"];
        let expansion = &structured["instruction"]["x"];

        assert_eq!(response["result"]["isError"], false);
        assert_eq!(structured["read_first"][0]["f"], "docs/COMPETITIVE.md");
        assert_eq!(expansion["o"]["arguments"]["file"], "docs/COMPETITIVE.md");
        assert!(expansion.get("top").is_none());
        assert!(expansion.get("rel").is_none());
        assert!(expansion.get("tests").is_none());
    }

    #[test]
    fn index_cache_serves_repeat_calls_without_reparsing() {
        let _guard = cache_lock();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        write(
            root.join("src/session.ts"),
            "export function createSession() {}\n",
        );
        let index = indexer::build_index(&root).unwrap();
        store::json_store::save_index(&root, &index).unwrap();

        let first = load_index_cached(&root).unwrap();
        assert_eq!(first.files.len(), 1);

        // Freshness checks stat the source files, not index.json, so a fresh
        // cache must keep answering even when the on-disk index is corrupted.
        fs::write(store::json_store::index_path(&root), "not json").unwrap();
        let second = load_index_cached(&root).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "expected the cached index to be reused"
        );

        // Touching a source file invalidates the cache; the corrupted disk
        // index now surfaces as a load error instead of silently stale data.
        write(
            root.join("src/session.ts"),
            "export function createSession() { return 1; }\n",
        );
        assert!(load_index_cached(&root).is_err());
    }
}
