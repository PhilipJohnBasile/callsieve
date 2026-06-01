use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::Serialize;

use crate::{indexer, output, query, store};

#[derive(Debug, Parser)]
#[command(
    name = "callsieve",
    version,
    about = "Local-first codebase retrieval for AI coding agents"
)]
pub struct Cli {
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build or replace the local CallSieve index.
    Index {
        path: PathBuf,

        /// Accepted for CLI stability. The current index command always rebuilds.
        #[arg(long)]
        refresh: bool,
    },

    /// List indexed symbols.
    Symbols {
        path: PathBuf,

        #[arg(long, default_value_t = 100)]
        limit: usize,
    },

    /// Find indexed symbols by name.
    Symbol {
        path: PathBuf,
        symbol_name: String,

        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Rank indexed files and symbols for a natural-language question.
    Query {
        path: PathBuf,
        question: String,

        #[arg(long, default_value_t = 10)]
        limit: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Build a compact read-first packet for a coding task.
    Context {
        path: PathBuf,
        task: String,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Build an agent-ready context packet agents should request before grep.
    AgentContext {
        path: PathBuf,
        task: String,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,
    },

    /// Estimate token savings versus a naive grep/read loop.
    Benchmark {
        path: PathBuf,
        task: String,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Run benchmark estimates across a JSON task suite.
    BenchmarkSuite {
        path: PathBuf,
        tasks: PathBuf,

        #[arg(long, default_value_t = 8)]
        limit: usize,

        #[arg(long, default_value_t = 2)]
        snippets_per_file: usize,

        #[arg(long)]
        no_snippets: bool,
    },

    /// Run a minimal MCP stdio server exposing CallSieve tools.
    Mcp,

    /// Show index statistics.
    Stats { path: PathBuf },
}

#[derive(Debug, Serialize)]
struct IndexOutput {
    command: &'static str,
    root: String,
    index: String,
    files: usize,
    symbols: usize,
    imports: usize,
    references: usize,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AgentContextOutput {
    instruction: &'static str,
    context: query::ContextOutput,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose)?;

    match cli.command {
        Command::Index { path, .. } => {
            let index = indexer::build_index(&path)?;
            let index_path = store::json_store::save_index(&path, &index)?;
            let output = IndexOutput {
                command: "index",
                root: root_label(&path),
                index: repo_relative_display(&path, &index_path),
                files: index.files.len(),
                symbols: index.symbols.len(),
                imports: index.imports.len(),
                references: index.references.len(),
                warnings: index.warnings,
            };
            output::json::print(&output)?;
        }
        Command::Symbols { path, limit } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::list_symbols(&path, &index, limit)?;
            output::json::print(&output)?;
        }
        Command::Symbol {
            path,
            symbol_name,
            limit,
        } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::find_symbol(&path, &index, &symbol_name, limit)?;
            output::json::print(&output)?;
        }
        Command::Query {
            path,
            question,
            limit,
            no_snippets,
        } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::run_query(&path, &index, &question, limit, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::Context {
            path,
            task,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let index = store::json_store::load_index(&path)?;
            let output =
                query::build_context(&path, &index, &task, limit, snippets_per_file, !no_snippets)?;
            output::json::print(&output)?;
        }
        Command::AgentContext {
            path,
            task,
            limit,
            snippets_per_file,
        } => {
            let index = store::json_store::load_index(&path)?;
            let context =
                query::build_context(&path, &index, &task, limit, snippets_per_file, true)?;
            let output = AgentContextOutput {
                instruction: "Use this read_first packet before grep. Read selected snippets and files first; grep only if the packet is insufficient.",
                context,
            };
            output::json::print(&output)?;
        }
        Command::Benchmark {
            path,
            task,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::benchmark_context(
                &path,
                &index,
                &task,
                limit,
                snippets_per_file,
                !no_snippets,
            )?;
            output::json::print(&output)?;
        }
        Command::BenchmarkSuite {
            path,
            tasks,
            limit,
            snippets_per_file,
            no_snippets,
        } => {
            let index = store::json_store::load_index(&path)?;
            let tasks_json = fs::read_to_string(&tasks)
                .with_context(|| format!("failed to read benchmark suite: {}", tasks.display()))?;
            let suite: query::BenchmarkSuiteInput = serde_json::from_str(&tasks_json)
                .with_context(|| format!("failed to parse benchmark suite: {}", tasks.display()))?;
            let output = query::benchmark_suite(
                &path,
                &index,
                suite,
                limit,
                snippets_per_file,
                !no_snippets,
            )?;
            output::json::print(&output)?;
        }
        Command::Mcp => {
            crate::mcp::run()?;
        }
        Command::Stats { path } => {
            let index = store::json_store::load_index(&path)?;
            let output = query::stats(&path, &index)?;
            output::json::print(&output)?;
        }
    }

    Ok(())
}

fn init_tracing(verbose: bool) -> Result<()> {
    if verbose {
        tracing_subscriber::fmt()
            .with_env_filter("callsieve=debug")
            .try_init()
            .ok();
    }
    Ok(())
}

fn root_label(path: &Path) -> String {
    if path == Path::new(".") {
        ".".to_string()
    } else {
        path.display().to_string()
    }
}

fn repo_relative_display(root: &Path, path: &Path) -> String {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    path.strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_all_commands() {
        Cli::try_parse_from(["callsieve", "index", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "symbols", "."]).unwrap();
        Cli::try_parse_from(["callsieve", "symbol", ".", "UserService"]).unwrap();
        Cli::try_parse_from(["callsieve", "query", ".", "where is auth handled?"]).unwrap();
        Cli::try_parse_from(["callsieve", "context", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from(["callsieve", "agent-context", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from(["callsieve", "benchmark", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from(["callsieve", "benchmark-suite", ".", "benchmarks/tasks.json"])
            .unwrap();
        Cli::try_parse_from(["callsieve", "mcp"]).unwrap();
        Cli::try_parse_from(["callsieve", "stats", "."]).unwrap();
    }
}
