mod cli;
mod indexer;
mod mcp;
mod output;
mod query;
mod store;

fn main() {
    if let Err(error) = cli::run() {
        output::json::print_error(&error);
        std::process::exit(1);
    }
}
