use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};

use crate::store::{FileRecord, LspServerStatus, ReferenceRecord, SymbolRecord};

use super::{language::Language, path_to_string};

const REQUEST_TIMEOUT: Duration = Duration::from_millis(10_000);
const MAX_LSP_SYMBOLS_PER_LANGUAGE: usize = 96;
const LSP_LANGUAGE_ENRICHMENT_TIMEOUT: Duration = Duration::from_millis(30_000);
const RUST_READY_RETRY_INTERVAL: Duration = Duration::from_millis(750);
const RUST_READY_TIMEOUT: Duration = Duration::from_millis(20_000);
const RUST_SEMANTIC_READY_TIMEOUT: Duration = Duration::from_millis(25_000);
const LSP_CONTENT_MODIFIED_RETRIES: usize = 3;

#[derive(Debug, Clone, Copy)]
struct ServerSpec {
    language: Language,
    language_name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    command_label: &'static str,
    language_id: &'static str,
    startup_settle_ms: u64,
}

pub fn server_statuses(files: &[FileRecord]) -> Vec<LspServerStatus> {
    let languages: BTreeSet<Language> = files
        .iter()
        .filter(|file| file.language.is_code())
        .map(|file| file.language)
        .collect();

    server_specs()
        .into_iter()
        .filter(|spec| languages.contains(&spec.language))
        .map(|spec| LspServerStatus {
            language: spec.language_name.to_string(),
            command: spec.command_label.to_string(),
            available: server_usable(spec),
            failure_reason: lsp_failure_reason(spec),
        })
        .collect()
}

pub fn enrich_references(
    root: &Path,
    file_contents: &[(String, String, Language, String)],
    files: &[FileRecord],
    symbols: &[SymbolRecord],
) -> Vec<ReferenceRecord> {
    let languages: BTreeSet<Language> = file_contents
        .iter()
        .map(|(_, _, language, _)| *language)
        .collect();
    let mut references = Vec::new();

    for spec in server_specs() {
        if !languages.contains(&spec.language) || !server_usable(spec) {
            continue;
        }

        match enrich_language_references(spec, root, file_contents, files, symbols) {
            Ok(mut language_references) => references.append(&mut language_references),
            Err(error) => tracing::debug!(
                "skipped LSP enrichment for {} via {}: {error:#}",
                spec.language_name,
                spec.command_label
            ),
        }
    }

    references
}

fn enrich_language_references(
    spec: ServerSpec,
    root: &Path,
    file_contents: &[(String, String, Language, String)],
    files: &[FileRecord],
    symbols: &[SymbolRecord],
) -> Result<Vec<ReferenceRecord>> {
    let language_files: Vec<&(String, String, Language, String)> = file_contents
        .iter()
        .filter(|(_, _, language, _)| *language == spec.language)
        .collect();
    if language_files.is_empty() {
        return Ok(Vec::new());
    }

    let deadline = Instant::now() + LSP_LANGUAGE_ENRICHMENT_TIMEOUT;
    let mut client = LspClient::spawn(spec)?;
    let root_uri = path_to_file_uri(root);
    client.request(
        "initialize",
        json!({
            "processId": null,
            "rootPath": path_to_lsp_path(root),
            "rootUri": root_uri,
            "workspaceFolders": [
                {
                    "uri": root_uri,
                    "name": root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("workspace")
                }
            ],
            "capabilities": {
                "textDocument": {
                    "documentSymbol": {
                        "dynamicRegistration": false,
                        "hierarchicalDocumentSymbolSupport": true
                    },
                    "references": {
                        "dynamicRegistration": false
                    },
                    "definition": {
                        "dynamicRegistration": false
                    },
                    "implementation": {
                        "dynamicRegistration": false
                    },
                    "typeDefinition": {
                        "dynamicRegistration": false
                    }
                },
                "workspace": {
                    "configuration": true,
                    "workspaceFolders": true
                },
                "general": {
                    "positionEncodings": ["utf-16"]
                }
            }
        }),
    )?;
    client.notify("initialized", json!({}))?;
    client.notify(
        "workspace/didChangeConfiguration",
        json!({
            "settings": did_change_configuration_settings(spec)
        }),
    )?;

    for (_, path, _, content) in &language_files {
        client.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": path_to_file_uri(&root.join(path)),
                    "languageId": language_id_for_path(spec, path),
                    "version": 1,
                    "text": content
                }
            }),
        )?;
    }
    let settle_duration = Duration::from_millis(spec.startup_settle_ms);
    thread::sleep(settle_duration.min(deadline.saturating_duration_since(Instant::now())));
    if Instant::now() >= deadline {
        tracing::debug!(
            "skipped LSP enrichment for {} after startup exceeded budget",
            spec.language_name
        );
        client.shutdown();
        return Ok(Vec::new());
    }

    let content_by_file_id: BTreeMap<&str, (&str, &str)> = language_files
        .iter()
        .map(|(file_id, path, _, content)| (file_id.as_str(), (path.as_str(), content.as_str())))
        .collect();
    let files_by_path: BTreeMap<&str, &FileRecord> = files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let symbol_file_paths: BTreeMap<&str, &str> = symbols
        .iter()
        .filter_map(|symbol| {
            files
                .iter()
                .find(|file| file.id == symbol.file_id)
                .map(|file| (symbol.id.as_str(), file.path.as_str()))
        })
        .collect();
    let lsp_symbol_queries = lsp_symbol_queries(
        spec,
        root,
        &mut client,
        &language_files,
        files,
        symbols,
        deadline,
    )?;
    wait_for_semantic_locations(spec, root, &mut client, &lsp_symbol_queries, deadline)?;

    let mut references = Vec::new();
    let mut queried_symbols = 0usize;
    let mut returned_locations = 0usize;
    let mut accepted_locations = 0usize;
    for query in lsp_symbol_queries.iter().take(MAX_LSP_SYMBOLS_PER_LANGUAGE) {
        if Instant::now() >= deadline {
            tracing::debug!(
                "stopped LSP reference queries for {} after hitting enrichment budget",
                spec.language_name
            );
            break;
        }
        queried_symbols += 1;
        let result = match client
            .request_with_retries("textDocument/references", lsp_reference_params(root, query))
        {
            Ok(result) => result,
            Err(error) => {
                tracing::debug!(
                    "skipped LSP references for {} at {}:{}: {error:#}",
                    query.target.name,
                    query.file_path,
                    query.position.line + 1
                );
                break;
            }
        };
        let locations = lsp_locations_from_result(result);
        returned_locations += locations.len();

        for location in locations {
            let context = LspReferenceContext {
                files_by_path: &files_by_path,
                symbols,
                content_by_file_id: &content_by_file_id,
                symbol_file_paths: &symbol_file_paths,
            };
            if let Some(reference) = reference_from_lsp_location(
                root,
                &query.target,
                &location,
                &context,
                "lsp_reference",
            ) {
                references.push(reference);
                accepted_locations += 1;
            }
        }
    }

    for (method, edge_source, kind) in [
        ("textDocument/definition", "lsp_definition", "definition"),
        (
            "textDocument/implementation",
            "lsp_implementation",
            "implementation",
        ),
        (
            "textDocument/typeDefinition",
            "lsp_type_definition",
            "type_definition",
        ),
    ] {
        for query in lsp_symbol_queries.iter().take(MAX_LSP_SYMBOLS_PER_LANGUAGE) {
            let result = match client.request_with_retries(method, lsp_position_params(root, query))
            {
                Ok(result) => result,
                Err(error) => {
                    tracing::debug!(
                        "skipped {method} for {} at {}:{}: {error:#}",
                        query.target.name,
                        query.file_path,
                        query.position.line + 1
                    );
                    break;
                }
            };

            for location in lsp_locations_from_result(result) {
                if let Some(reference) = reference_to_lsp_target_location(
                    root,
                    query,
                    &location,
                    &files_by_path,
                    symbols,
                    edge_source,
                    kind,
                ) {
                    references.push(reference);
                }
            }
        }
    }

    tracing::debug!(
        "LSP enrichment for {} queried {queried_symbols} symbols, returned {returned_locations} locations, accepted {accepted_locations} references",
        spec.language_name
    );
    client.shutdown();
    Ok(references)
}

#[derive(Debug, Clone)]
struct LspSymbolQuery {
    file_path: String,
    position: LspPosition,
    target: LspReferenceTarget,
}

#[derive(Debug, Clone)]
struct LspReferenceTarget {
    name: String,
    symbol_id: Option<String>,
    path: Option<String>,
    start_line: usize,
}

fn lsp_symbol_queries(
    spec: ServerSpec,
    root: &Path,
    client: &mut LspClient,
    language_files: &[&(String, String, Language, String)],
    files: &[FileRecord],
    symbols: &[SymbolRecord],
    deadline: Instant,
) -> Result<Vec<LspSymbolQuery>> {
    let files_by_path: BTreeMap<&str, &FileRecord> = files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let document_symbol_deadline = capped_deadline(
        Instant::now() + document_symbol_ready_timeout(spec),
        deadline,
    );

    loop {
        let mut queries = Vec::new();
        let mut seen = BTreeSet::new();
        let mut document_symbols = 0usize;

        for (_, path, _, _) in language_files {
            if Instant::now() >= deadline {
                break;
            }
            let result = client.request_with_retries(
                "textDocument/documentSymbol",
                json!({
                    "textDocument": {
                        "uri": path_to_file_uri(&root.join(path))
                    }
                }),
            )?;
            let Some(values) = result.and_then(|value| value.as_array().cloned()) else {
                continue;
            };
            let Some(file) = files_by_path.get(path.as_str()) else {
                continue;
            };
            let mut file_queries = Vec::new();
            collect_document_symbol_queries(path, file, &values, symbols, &mut file_queries);
            document_symbols += file_queries.len();
            for query in file_queries {
                let key = symbol_query_key(&query);
                if seen.insert(key) {
                    queries.push(query);
                }
            }
        }

        if !queries.is_empty() || Instant::now() >= document_symbol_deadline {
            if queries.is_empty() {
                queries = fallback_symbol_queries(spec, language_files, files, symbols);
            }

            tracing::debug!(
                "LSP document symbols for {} produced {document_symbols} query points",
                spec.language_name
            );
            return Ok(queries);
        }

        tracing::debug!(
            "waiting for {} document symbols; last query points: {document_symbols}",
            spec.language_name
        );
        thread::sleep(RUST_READY_RETRY_INTERVAL);
    }
}

fn collect_document_symbol_queries(
    path: &str,
    file: &FileRecord,
    values: &[Value],
    symbols: &[SymbolRecord],
    queries: &mut Vec<LspSymbolQuery>,
) {
    for value in values {
        if let Some(query) = document_symbol_query(path, file, value, symbols) {
            queries.push(query);
        }

        if let Some(children) = value.get("children").and_then(Value::as_array) {
            collect_document_symbol_queries(path, file, children, symbols, queries);
        }
    }
}

fn document_symbol_query(
    path: &str,
    file: &FileRecord,
    value: &Value,
    symbols: &[SymbolRecord],
) -> Option<LspSymbolQuery> {
    let name = value.get("name").and_then(Value::as_str)?.to_string();
    let position = lsp_position_from_value(
        value
            .pointer("/selectionRange/start")
            .or_else(|| value.pointer("/location/range/start"))
            .or_else(|| value.pointer("/range/start"))?,
    )?;
    let line = position.line + 1;
    let matched_symbol = symbols
        .iter()
        .filter(|symbol| symbol.file_id == file.id && symbol.name == name)
        .min_by_key(|symbol| symbol.start_line.abs_diff(line));

    Some(LspSymbolQuery {
        file_path: path.to_string(),
        position,
        target: LspReferenceTarget {
            name,
            symbol_id: matched_symbol.map(|symbol| symbol.id.clone()),
            path: Some(path.to_string()),
            start_line: matched_symbol
                .map(|symbol| symbol.start_line)
                .unwrap_or(line),
        },
    })
}

fn fallback_symbol_queries(
    spec: ServerSpec,
    language_files: &[&(String, String, Language, String)],
    files: &[FileRecord],
    symbols: &[SymbolRecord],
) -> Vec<LspSymbolQuery> {
    let content_by_file_id: BTreeMap<&str, (&str, &str)> = language_files
        .iter()
        .map(|(file_id, path, _, content)| (file_id.as_str(), (path.as_str(), content.as_str())))
        .collect();
    let symbol_file_paths: BTreeMap<&str, &str> = symbols
        .iter()
        .filter_map(|symbol| {
            files
                .iter()
                .find(|file| file.id == symbol.file_id)
                .map(|file| (symbol.id.as_str(), file.path.as_str()))
        })
        .collect();

    symbols
        .iter()
        .filter(|symbol| symbol.language == spec.language)
        .filter_map(|symbol| {
            let (path, content) = content_by_file_id.get(symbol.file_id.as_str())?;
            let position = symbol_position(symbol, content)?;
            Some(LspSymbolQuery {
                file_path: (*path).to_string(),
                position,
                target: LspReferenceTarget {
                    name: symbol.name.clone(),
                    symbol_id: Some(symbol.id.clone()),
                    path: symbol_file_paths
                        .get(symbol.id.as_str())
                        .map(|path| (*path).to_string()),
                    start_line: symbol.start_line,
                },
            })
        })
        .collect()
}

fn lsp_position_from_value(value: &Value) -> Option<LspPosition> {
    Some(LspPosition {
        line: value.get("line").and_then(Value::as_u64)? as usize,
        character: value.get("character").and_then(Value::as_u64)? as usize,
    })
}

fn symbol_query_key(query: &LspSymbolQuery) -> (String, String, usize, usize) {
    (
        query.file_path.clone(),
        query.target.name.clone(),
        query.position.line,
        query.position.character,
    )
}

fn lsp_position_params(root: &Path, query: &LspSymbolQuery) -> Value {
    json!({
        "textDocument": {
            "uri": path_to_file_uri(&root.join(&query.file_path))
        },
        "position": {
            "line": query.position.line,
            "character": query.position.character
        }
    })
}

fn lsp_reference_params(root: &Path, query: &LspSymbolQuery) -> Value {
    json!({
        "textDocument": {
            "uri": path_to_file_uri(&root.join(&query.file_path))
        },
        "position": {
            "line": query.position.line,
            "character": query.position.character
        },
        "context": {
            "includeDeclaration": true
        }
    })
}

fn wait_for_semantic_locations(
    spec: ServerSpec,
    root: &Path,
    client: &mut LspClient,
    queries: &[LspSymbolQuery],
    deadline: Instant,
) -> Result<()> {
    if spec.language != Language::Rust || queries.is_empty() {
        return Ok(());
    }

    let probe_count = queries.len().min(16);
    let minimum_ready = probe_count.min(12);
    if minimum_ready == 0 {
        return Ok(());
    }

    let semantic_deadline = capped_deadline(Instant::now() + RUST_SEMANTIC_READY_TIMEOUT, deadline);
    loop {
        let mut ready_queries = 0usize;
        let mut returned_locations = 0usize;

        for query in queries.iter().take(probe_count) {
            if Instant::now() >= semantic_deadline {
                return Ok(());
            }
            let result = client.request_with_retries(
                "textDocument/references",
                lsp_reference_params(root, query),
            )?;
            let locations = lsp_locations_from_result(result);
            if !locations.is_empty() {
                ready_queries += 1;
                returned_locations += locations.len();
            }
        }

        if ready_queries >= minimum_ready {
            tracing::debug!(
                "rust-analyzer semantic probe ready: {ready_queries}/{probe_count} symbols, {returned_locations} locations"
            );
            return Ok(());
        }

        if Instant::now() >= semantic_deadline {
            tracing::debug!(
                "rust-analyzer semantic probe timed out: {ready_queries}/{probe_count} symbols, {returned_locations} locations"
            );
            return Ok(());
        }

        tracing::debug!(
            "waiting for rust-analyzer semantic locations: {ready_queries}/{probe_count} symbols, {returned_locations} locations"
        );
        thread::sleep(RUST_READY_RETRY_INTERVAL);
    }
}

fn lsp_locations_from_result(result: Option<Value>) -> Vec<Value> {
    match result {
        Some(Value::Array(locations)) => locations,
        Some(Value::Object(object))
            if object.contains_key("uri") || object.contains_key("targetUri") =>
        {
            vec![Value::Object(object)]
        }
        _ => Vec::new(),
    }
}

struct LspReferenceContext<'a> {
    files_by_path: &'a BTreeMap<&'a str, &'a FileRecord>,
    symbols: &'a [SymbolRecord],
    content_by_file_id: &'a BTreeMap<&'a str, (&'a str, &'a str)>,
    symbol_file_paths: &'a BTreeMap<&'a str, &'a str>,
}

fn reference_from_lsp_location(
    root: &Path,
    target: &LspReferenceTarget,
    location: &Value,
    context: &LspReferenceContext<'_>,
    edge_source: &str,
) -> Option<ReferenceRecord> {
    let uri = location
        .get("uri")
        .or_else(|| location.get("targetUri"))
        .and_then(Value::as_str)?;
    let line = location
        .pointer("/range/start/line")
        .or_else(|| location.pointer("/targetSelectionRange/start/line"))
        .or_else(|| location.pointer("/targetRange/start/line"))
        .and_then(Value::as_u64)? as usize
        + 1;
    let absolute_path = file_uri_to_path(uri)?;
    let absolute_path = absolute_path.canonicalize().unwrap_or(absolute_path);
    let relative_path = absolute_path
        .strip_prefix(root)
        .ok()
        .map(path_to_string)
        .unwrap_or_else(|| path_to_string(&absolute_path));
    let source_file = context.files_by_path.get(relative_path.as_str())?;
    let target_path = target
        .symbol_id
        .as_deref()
        .and_then(|symbol_id| context.symbol_file_paths.get(symbol_id).copied())
        .map(str::to_string)
        .or_else(|| target.path.clone());

    if target_path.as_deref() == Some(source_file.path.as_str()) && line == target.start_line {
        return None;
    }

    let kind = context
        .content_by_file_id
        .get(source_file.id.as_str())
        .map(|(_, content)| reference_kind(content, line, &target.name))
        .unwrap_or_else(|| "reference".to_string());

    Some(ReferenceRecord {
        file_id: source_file.id.clone(),
        source_path: source_file.path.clone(),
        source_symbol_id: source_symbol_for_line(&source_file.id, line, context.symbols)
            .map(|symbol| symbol.id.clone()),
        target_name: target.name.clone(),
        target_symbol_id: target.symbol_id.clone(),
        target_path,
        kind,
        line,
        edge_source: edge_source.to_string(),
        confidence: 1.0,
        lsp_method: Some(lsp_method_name(edge_source).to_string()),
        // a range equal to `line` is implied; omitted to keep the index small
        source_range: None,
        target_range: Some([target.start_line, target.start_line]),
    })
}

fn reference_to_lsp_target_location(
    root: &Path,
    query: &LspSymbolQuery,
    location: &Value,
    files_by_path: &BTreeMap<&str, &FileRecord>,
    symbols: &[SymbolRecord],
    edge_source: &str,
    kind: &str,
) -> Option<ReferenceRecord> {
    let uri = location
        .get("uri")
        .or_else(|| location.get("targetUri"))
        .and_then(Value::as_str)?;
    let line = location
        .pointer("/range/start/line")
        .or_else(|| location.pointer("/targetSelectionRange/start/line"))
        .or_else(|| location.pointer("/targetRange/start/line"))
        .and_then(Value::as_u64)? as usize
        + 1;
    let absolute_path = file_uri_to_path(uri)?;
    let absolute_path = absolute_path.canonicalize().unwrap_or(absolute_path);
    let target_path = absolute_path
        .strip_prefix(root)
        .ok()
        .map(path_to_string)
        .unwrap_or_else(|| path_to_string(&absolute_path));
    let source_file = files_by_path.get(query.file_path.as_str())?;
    let target_file = files_by_path.get(target_path.as_str())?;

    if source_file.path == target_file.path && line == query.target.start_line {
        return None;
    }

    let target_symbol = symbols
        .iter()
        .filter(|symbol| symbol.file_id == target_file.id && symbol.name == query.target.name)
        .min_by_key(|symbol| symbol.start_line.abs_diff(line));

    Some(ReferenceRecord {
        file_id: source_file.id.clone(),
        source_path: source_file.path.clone(),
        source_symbol_id: query.target.symbol_id.clone(),
        target_name: query.target.name.clone(),
        target_symbol_id: target_symbol
            .map(|symbol| symbol.id.clone())
            .or_else(|| query.target.symbol_id.clone()),
        target_path: Some(target_file.path.clone()),
        kind: kind.to_string(),
        line: query.target.start_line,
        edge_source: edge_source.to_string(),
        confidence: 1.0,
        lsp_method: Some(lsp_method_name(edge_source).to_string()),
        // a range equal to `line` is implied; omitted to keep the index small
        source_range: None,
        target_range: Some([line, line]),
    })
}

fn lsp_method_name(edge_source: &str) -> &'static str {
    match edge_source {
        "lsp_reference" | "lsp" => "textDocument/references",
        "lsp_definition" => "textDocument/definition",
        "lsp_implementation" => "textDocument/implementation",
        "lsp_type_definition" => "textDocument/typeDefinition",
        _ => "textDocument/unknown",
    }
}

fn reference_kind(content: &str, line: usize, target_name: &str) -> String {
    let Some(line_text) = content.lines().nth(line.saturating_sub(1)) else {
        return "reference".to_string();
    };
    let Some(index) = line_text.find(target_name) else {
        return "reference".to_string();
    };
    let after_name = &line_text[index + target_name.len()..];
    if after_name.trim_start().starts_with('(') {
        "call".to_string()
    } else {
        "reference".to_string()
    }
}

fn source_symbol_for_line<'a>(
    file_id: &str,
    line: usize,
    symbols: &'a [SymbolRecord],
) -> Option<&'a SymbolRecord> {
    symbols
        .iter()
        .filter(|symbol| {
            symbol.file_id == file_id && symbol.start_line <= line && symbol.end_line >= line
        })
        .min_by_key(|symbol| symbol.end_line.saturating_sub(symbol.start_line))
}

#[derive(Debug, Clone, Copy)]
struct LspPosition {
    line: usize,
    character: usize,
}

fn symbol_position(symbol: &SymbolRecord, content: &str) -> Option<LspPosition> {
    let line = symbol.start_line.checked_sub(1)?;
    let line_text = content.lines().nth(line)?;
    let byte_index = line_text.find(&symbol.name).unwrap_or_default();
    Some(LspPosition {
        line,
        character: byte_to_utf16_character(line_text, byte_index),
    })
}

fn byte_to_utf16_character(line: &str, byte_index: usize) -> usize {
    line[..byte_index.min(line.len())].encode_utf16().count()
}

struct LspClient {
    child: Child,
    stdin: ChildStdin,
    receiver: Receiver<Value>,
    next_id: u64,
    spec: ServerSpec,
}

impl LspClient {
    fn spawn(spec: ServerSpec) -> Result<Self> {
        let mut child = Command::new(spec.command)
            .args(spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to spawn {}", spec.command_label))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("missing language server stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing language server stdout"))?;
        let receiver = spawn_reader(stdout);

        Ok(Self {
            child,
            stdin,
            receiver,
            next_id: 1,
            spec,
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Option<Value>> {
        let id = self.next_id;
        self.next_id += 1;
        send_message(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params
            }),
        )?;

        loop {
            match self.receiver.recv_timeout(REQUEST_TIMEOUT) {
                Ok(message) => {
                    if self.handle_server_message(&message)? {
                        continue;
                    }
                    if message.get("id").and_then(Value::as_u64) != Some(id) {
                        tracing::debug!(
                            "ignored LSP message while waiting for {method}: {message}"
                        );
                        continue;
                    }
                    if let Some(error) = message.get("error") {
                        return Err(anyhow!("LSP request {method} failed: {error}"));
                    }
                    return Ok(message.get("result").cloned());
                }
                Err(RecvTimeoutError::Timeout) => {
                    let _ = self.child.kill();
                    return Err(anyhow!("LSP request {method} timed out"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!("language server exited during {method}"));
                }
            }
        }
    }

    fn request_with_retries(&mut self, method: &str, params: Value) -> Result<Option<Value>> {
        for attempt in 0..LSP_CONTENT_MODIFIED_RETRIES {
            match self.request(method, params.clone()) {
                Ok(result) => return Ok(result),
                Err(error)
                    if is_lsp_content_modified(&error)
                        && attempt + 1 < LSP_CONTENT_MODIFIED_RETRIES =>
                {
                    tracing::debug!("retrying {method} after rust-analyzer content modified");
                    thread::sleep(RUST_READY_RETRY_INTERVAL);
                }
                Err(error) => return Err(error),
            }
        }

        unreachable!("LSP retry loop returns from every branch")
    }

    fn handle_server_message(&mut self, message: &Value) -> Result<bool> {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(false);
        };
        let Some(id) = message.get("id").cloned() else {
            return Ok(true);
        };

        let result = match method {
            "workspace/configuration" => {
                let items = message
                    .pointer("/params/items")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                Value::Array(
                    items
                        .iter()
                        .map(|item| configuration_for_item(self.spec, item))
                        .collect(),
                )
            }
            "workspace/applyEdit" => json!({ "applied": false }),
            "client/registerCapability" | "client/unregisterCapability" => Value::Null,
            "window/workDoneProgress/create" | "workspace/diagnostic/refresh" => Value::Null,
            _ => {
                tracing::debug!("answering unsupported LSP server request {method} with null");
                Value::Null
            }
        };

        send_message(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            }),
        )?;
        Ok(true)
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        send_message(
            &mut self.stdin,
            &json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params
            }),
        )
    }

    fn shutdown(&mut self) {
        let _ = self.request("shutdown", Value::Null);
        let _ = self.notify("exit", Value::Null);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_reader(stdout: ChildStdout) -> Receiver<Value> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Ok(Some(message)) = read_message(&mut reader) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    receiver
}

fn send_message(stdin: &mut ChildStdin, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
    stdin.write_all(&body)?;
    stdin.flush()?;
    Ok(())
}

fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<Option<Value>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = trimmed
            .strip_prefix("Content-Length:")
            .or_else(|| trimmed.strip_prefix("content-length:"))
        {
            content_length = Some(value.trim().parse::<usize>()?);
        }
    }

    let content_length = content_length.ok_or_else(|| anyhow!("missing LSP content length"))?;
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

fn path_to_file_uri(path: &Path) -> String {
    let path = path_to_lsp_path(path);
    if cfg!(windows) && !path.starts_with('/') {
        format!("file:///{}", percent_encode_path(&path))
    } else {
        format!("file://{}", percent_encode_path(&path))
    }
}

fn path_to_lsp_path(path: &Path) -> String {
    let mut path = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/");
    if cfg!(windows) {
        if let Some(stripped) = path.strip_prefix("//?/UNC/") {
            path = format!("//{stripped}");
        } else if let Some(stripped) = path.strip_prefix("//?/") {
            path = stripped.to_string();
        }
    }
    path
}

fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let decoded = percent_decode_path(path);
    if cfg!(windows) {
        let decoded = decoded.trim_start_matches('/');
        Some(PathBuf::from(decoded.replace('/', "\\")))
    } else {
        Some(PathBuf::from(decoded))
    }
}

fn percent_encode_path(path: &str) -> String {
    path.bytes()
        .flat_map(|byte| match byte {
            b' ' => "%20".bytes().collect::<Vec<_>>(),
            b'#' => "%23".bytes().collect(),
            b'%' => "%25".bytes().collect(),
            b'?' => "%3F".bytes().collect(),
            _ => vec![byte],
        })
        .map(char::from)
        .collect()
}

fn percent_decode_path(path: &str) -> String {
    let mut output = String::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&path[index + 1..index + 3], 16)
        {
            output.push(char::from(value));
            index += 3;
        } else {
            output.push(char::from(bytes[index]));
            index += 1;
        }
    }
    output
}

fn language_id_for_path(spec: ServerSpec, path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("tsx") => "typescriptreact",
        Some("jsx") => "javascriptreact",
        _ => spec.language_id,
    }
}

fn is_lsp_content_modified(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}").to_ascii_lowercase();
    message.contains("content modified") || message.contains("\"code\":-32801")
}

fn document_symbol_ready_timeout(spec: ServerSpec) -> Duration {
    if spec.language == Language::Rust {
        RUST_READY_TIMEOUT
    } else {
        Duration::ZERO
    }
}

fn capped_deadline(left: Instant, right: Instant) -> Instant {
    if left <= right { left } else { right }
}

fn did_change_configuration_settings(spec: ServerSpec) -> Value {
    if spec.language == Language::Rust {
        json!({
            "rust-analyzer": rust_analyzer_configuration()
        })
    } else {
        json!({})
    }
}

fn configuration_for_item(spec: ServerSpec, item: &Value) -> Value {
    if spec.language == Language::Rust
        && item
            .get("section")
            .and_then(Value::as_str)
            .is_some_and(|section| section == "rust-analyzer")
    {
        return rust_analyzer_configuration();
    }

    json!({})
}

fn rust_analyzer_configuration() -> Value {
    json!({
        "cargo": {
            "buildScripts": {
                "enable": true
            },
            "features": "all"
        },
        "procMacro": {
            "enable": true
        },
        "checkOnSave": false
    })
}

fn server_specs() -> [ServerSpec; 16] {
    [
        ServerSpec {
            language: Language::TypeScript,
            language_name: "typescript",
            command: "typescript-language-server",
            args: &["--stdio"],
            command_label: "typescript-language-server --stdio",
            language_id: "typescript",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::JavaScript,
            language_name: "javascript",
            command: "typescript-language-server",
            args: &["--stdio"],
            command_label: "typescript-language-server --stdio",
            language_id: "javascript",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Python,
            language_name: "python",
            command: "pyright-langserver",
            args: &["--stdio"],
            command_label: "pyright-langserver --stdio",
            language_id: "python",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Rust,
            language_name: "rust",
            command: "rust-analyzer",
            args: &[],
            command_label: "rust-analyzer",
            language_id: "rust",
            startup_settle_ms: 10_000,
        },
        ServerSpec {
            language: Language::Php,
            language_name: "php",
            command: "intelephense",
            args: &["--stdio"],
            command_label: "intelephense --stdio",
            language_id: "php",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Go,
            language_name: "go",
            command: "gopls",
            args: &[],
            command_label: "gopls",
            language_id: "go",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::C,
            language_name: "c",
            command: "clangd",
            args: &[],
            command_label: "clangd",
            language_id: "c",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Cpp,
            language_name: "cpp",
            command: "clangd",
            args: &[],
            command_label: "clangd",
            language_id: "cpp",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Ruby,
            language_name: "ruby",
            command: "ruby-lsp",
            args: &[],
            command_label: "ruby-lsp",
            language_id: "ruby",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Lua,
            language_name: "lua",
            command: "lua-language-server",
            args: &[],
            command_label: "lua-language-server",
            language_id: "lua",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::CSharp,
            language_name: "csharp",
            command: "csharp-ls",
            args: &[],
            command_label: "csharp-ls",
            language_id: "csharp",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Java,
            language_name: "java",
            command: "jdtls",
            args: &[],
            command_label: "jdtls",
            language_id: "java",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Kotlin,
            language_name: "kotlin",
            command: "kotlin-language-server",
            args: &[],
            command_label: "kotlin-language-server",
            language_id: "kotlin",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Swift,
            language_name: "swift",
            command: "sourcekit-lsp",
            args: &[],
            command_label: "sourcekit-lsp",
            language_id: "swift",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Scala,
            language_name: "scala",
            command: "metals",
            args: &[],
            command_label: "metals",
            language_id: "scala",
            startup_settle_ms: 1_000,
        },
        ServerSpec {
            language: Language::Dart,
            language_name: "dart",
            command: "dart",
            args: &["language-server", "--protocol=lsp"],
            command_label: "dart language-server --protocol=lsp",
            language_id: "dart",
            startup_settle_ms: 1_000,
        },
    ]
}

fn server_usable(spec: ServerSpec) -> bool {
    command_on_path(spec.command) && command_version_succeeds(spec.command)
}

fn lsp_failure_reason(spec: ServerSpec) -> Option<String> {
    if !command_on_path(spec.command) {
        return Some(format!("{} is not on PATH", spec.command));
    }
    if !command_version_succeeds(spec.command) {
        return Some(format!(
            "{} --version did not complete successfully",
            spec.command
        ));
    }
    None
}

fn command_on_path(command: &str) -> bool {
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    let extensions: Vec<String> = if cfg!(windows) {
        env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
            .split(';')
            .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };

    env::split_paths(&paths).any(|dir| {
        if cfg!(windows) {
            extensions.iter().any(|extension| {
                dir.join(format!("{command}.{extension}")).is_file() || dir.join(command).is_file()
            })
        } else {
            dir.join(command).is_file()
        }
    })
}

fn command_version_succeeds(command: &str) -> bool {
    let Ok(mut child) = Command::new(command)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_millis(700);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_round_trip_handles_encoded_chars() {
        let path = env::temp_dir()
            .join("call sieve#one")
            .join("src")
            .join("lib.rs");
        let uri = path_to_file_uri(&path);

        assert!(uri.contains("%20"));
        assert!(uri.contains("%23"));
        assert_eq!(file_uri_to_path(&uri).unwrap(), path);
    }

    #[test]
    fn uri_strips_windows_verbatim_prefix() {
        if !cfg!(windows) {
            return;
        }

        let uri = path_to_file_uri(Path::new(r"\\?\C:\tmp\call sieve#one\src\lib.rs"));

        assert!(uri.starts_with("file:///C:/tmp/"));
        assert!(!uri.contains("?/"));
        assert!(uri.contains("%20"));
    }

    #[test]
    fn symbol_position_uses_utf16_character_offset() {
        let symbol = SymbolRecord {
            id: "symbol:src/main.py:1:hello".to_string(),
            file_id: "file:src/main.py".to_string(),
            name: "hello".to_string(),
            kind: "function".to_string(),
            language: Language::Python,
            start_line: 1,
            end_line: 2,
            visibility: "local".to_string(),
            parent: None,
            signature: "def hello()".to_string(),
            doc: None,
        };
        let position = symbol_position(&symbol, "é = 1\ndef hello():\n    pass\n").unwrap();

        assert_eq!(position.line, 0);
        assert_eq!(position.character, 0);
        let second_line_symbol = SymbolRecord {
            start_line: 2,
            ..symbol
        };
        let position =
            symbol_position(&second_line_symbol, "é = 1\ndef hello():\n    pass\n").unwrap();
        assert_eq!(position.character, 4);
    }

    #[test]
    fn reference_kind_distinguishes_calls_from_plain_references() {
        assert_eq!(
            reference_kind("let value = tokenFor(user);\n", 1, "tokenFor"),
            "call"
        );
        assert_eq!(
            reference_kind("let value = tokenFor;\n", 1, "tokenFor"),
            "reference"
        );
    }

    #[test]
    fn lsp_failure_reason_records_missing_server() {
        let spec = ServerSpec {
            language: Language::Rust,
            language_name: "rust",
            command: "callsieve-definitely-missing-lsp",
            args: &[],
            command_label: "callsieve-definitely-missing-lsp",
            language_id: "rust",
            startup_settle_ms: 0,
        };

        assert!(!server_usable(spec));
        assert!(lsp_failure_reason(spec).unwrap().contains("is not on PATH"));
    }
}
