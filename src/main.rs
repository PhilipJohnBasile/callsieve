//! callsieve CLI — a thin binary over the `callsieve` library (see `src/lib.rs`).

use callsieve::{cli, output};

fn main() {
    if let Err(error) = cli::run() {
        output::json::print_error(&error);
        std::process::exit(1);
    }
}
