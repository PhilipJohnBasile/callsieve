//! callsieve — code retrieval & bug localization, as a library.
//!
//! Exposes the same engine the `callsieve` CLI uses, so other Rust tools (e.g. `merle`) can embed it
//! directly instead of shelling out to the binary. The binary (`src/main.rs`) is a thin wrapper over
//! `cli::run`.

pub mod bench_public;
pub mod cli;
pub mod indexer;
pub mod mcp;
pub mod output;
pub mod query;
pub mod store;
