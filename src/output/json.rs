use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Error, Result};
use serde::Serialize;

static PRETTY_OUTPUT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    message: String,
}

pub fn print<T: Serialize>(value: &T) -> Result<()> {
    if PRETTY_OUTPUT.load(Ordering::Relaxed) {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", serde_json::to_string(value)?);
    }
    Ok(())
}

pub fn set_pretty(pretty: bool) {
    PRETTY_OUTPUT.store(pretty, Ordering::Relaxed);
}

pub fn print_error(error: &Error) {
    let envelope = ErrorEnvelope {
        error: ErrorBody {
            message: error.to_string(),
        },
    };

    let encoded = if PRETTY_OUTPUT.load(Ordering::Relaxed) {
        serde_json::to_string_pretty(&envelope)
    } else {
        serde_json::to_string(&envelope)
    };

    match encoded {
        Ok(json) => println!("{json}"),
        Err(_) => println!(r#"{{"error":{{"message":"unexpected error"}}}}"#),
    }
}
