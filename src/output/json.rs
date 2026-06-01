use anyhow::{Error, Result};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    message: String,
}

pub fn print<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub fn print_error(error: &Error) {
    let envelope = ErrorEnvelope {
        error: ErrorBody {
            message: error.to_string(),
        },
    };

    match serde_json::to_string_pretty(&envelope) {
        Ok(json) => println!("{json}"),
        Err(_) => println!(r#"{{"error":{{"message":"unexpected error"}}}}"#),
    }
}
