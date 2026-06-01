use std::path::{Path, PathBuf};

use anyhow::Result;
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
    warnings: Vec<String>,
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
        Cli::try_parse_from(["callsieve", "benchmark", ".", "change token expiry"]).unwrap();
        Cli::try_parse_from(["callsieve", "stats", "."]).unwrap();
    }
}
