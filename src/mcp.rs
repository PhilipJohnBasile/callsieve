use std::{
    io::{self, BufRead, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{query, store};

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
                "description": "Preferred first tool for codebase discovery. Build a compact CallSieve read-first packet before grep or broad file reads.",
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
                        }
                    },
                    "required": ["path", "task"]
                }
            }
        ]
    })
}

fn call_tool(params: Option<Value>) -> Result<Value> {
    let params = params.ok_or_else(|| anyhow!("tools/call requires params"))?;
    let name = required_str(&params, "name")?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "callsieve_context" => Ok(tool_execution_result(execute_context(&arguments))),
        "callsieve_symbol" => Ok(tool_execution_result(execute_symbol(&arguments))),
        "callsieve_stats" => Ok(tool_execution_result(execute_stats(&arguments))),
        "callsieve_status" => Ok(tool_execution_result(execute_status(&arguments))),
        "callsieve_trace_check" => Ok(tool_execution_result(execute_trace_check(&arguments))),
        "callsieve_benchmark" => Ok(tool_execution_result(execute_benchmark(&arguments))),
        name => Err(anyhow!("unknown tool: {name}")),
    }
}

fn execute_context(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let task = required_str(arguments, "task")?;
    let limit = optional_usize(arguments, "limit", 8)?;
    let snippets_per_file = optional_usize(arguments, "snippets_per_file", 2)?;
    let include_snippets = !optional_bool(arguments, "no_snippets", false)?;
    let index = store::json_store::load_index(&path)?;
    let output = query::build_context(
        &path,
        &index,
        task,
        limit,
        snippets_per_file,
        include_snippets,
    )?;

    let mut value = serde_json::to_value(output)?;
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "trace_event".to_string(),
            json!({
                "tool": "callsieve_context",
                "policy": "first_codebase_discovery_tool",
                "called_at": now_unix_seconds()
            }),
        );
    }

    Ok(value)
}

fn execute_symbol(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let symbol_name = required_str(arguments, "symbol_name")?;
    let limit = optional_usize(arguments, "limit", 20)?;
    let index = store::json_store::load_index(&path)?;
    let output = query::find_symbol(&path, &index, symbol_name, limit)?;

    Ok(serde_json::to_value(output)?)
}

fn execute_stats(arguments: &Value) -> Result<Value> {
    let path = repo_path(arguments)?;
    let index = store::json_store::load_index(&path)?;
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
    let index = store::json_store::load_index(&path)?;
    let output = query::benchmark_context(
        &path,
        &index,
        task,
        limit,
        snippets_per_file,
        include_snippets,
    )?;

    Ok(serde_json::to_value(output)?)
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

fn tool_execution_result(result: Result<Value>) -> Value {
    match result {
        Ok(structured_content) => {
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
    }

    #[test]
    fn context_tool_returns_structured_content() {
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
        assert_eq!(
            response["result"]["structuredContent"]["read_first"][0]["file"],
            "src/auth/session.ts"
        );
    }
}
