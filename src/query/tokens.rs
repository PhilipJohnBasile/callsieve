//! Token counting for budget enforcement and proof math.
//!
//! CallSieve's promise is token savings, so the count that drives budget
//! enforcement and every `benchmark` / `context_payload_reduction` /
//! `proof-report` number matters. The default build stays dependency-light and
//! uses a deterministic byte heuristic (~4 bytes/token). Built with
//! `--features tokenizers`, real provider tokenizers give exact, per-model
//! counts without changing the zero-cloud, deterministic architecture (a real
//! BPE tokenizer is still a pure function of its input).
//!
//! The active tokenizer is process-global config, set once from the CLI before
//! any retrieval runs, so the ~30 existing `estimate_tokens` /
//! `value_estimated_tokens` call sites need no per-call plumbing.

use std::sync::atomic::{AtomicU8, Ordering};

/// Which tokenizer drives token counts for this process. The CLI surface is
/// stable regardless of build features; real tokenizers fall back to the byte
/// heuristic (with a one-time warning) when the `tokenizers` feature is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TokenizerKind {
    /// Dependency-light `bytes / 4` heuristic. Default, ships today.
    #[default]
    Heuristic,
    /// OpenAI `o200k_base` (GPT-4o / GPT-5 family). Requires `tokenizers`.
    O200k,
    /// OpenAI `cl100k_base` (GPT-4 / 3.5 family). Requires `tokenizers`.
    Cl100k,
}

impl TokenizerKind {
    fn as_u8(self) -> u8 {
        match self {
            TokenizerKind::Heuristic => 0,
            TokenizerKind::O200k => 1,
            TokenizerKind::Cl100k => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => TokenizerKind::O200k,
            2 => TokenizerKind::Cl100k,
            _ => TokenizerKind::Heuristic,
        }
    }

    /// Stable name used by the CLI flag and proof artifacts.
    pub fn name(self) -> &'static str {
        match self {
            TokenizerKind::Heuristic => "heuristic",
            TokenizerKind::O200k => "o200k",
            TokenizerKind::Cl100k => "cl100k",
        }
    }

    /// Parses a CLI value. Returns `None` for unknown names.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "heuristic" => Some(TokenizerKind::Heuristic),
            "o200k" => Some(TokenizerKind::O200k),
            "cl100k" => Some(TokenizerKind::Cl100k),
            _ => None,
        }
    }
}

static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// Selects the process-wide tokenizer. Call once at startup. When a real
/// tokenizer is requested but the `tokenizers` feature is not built, this keeps
/// the heuristic and warns once on stderr so counts are never silently wrong.
pub fn set_active(kind: TokenizerKind) {
    #[cfg(not(feature = "tokenizers"))]
    if kind != TokenizerKind::Heuristic {
        eprintln!(
            "callsieve: tokenizer '{}' requested but this binary was built without the \
             'tokenizers' feature; falling back to the byte heuristic. Rebuild with \
             `--features tokenizers` for exact counts.",
            kind.name()
        );
        ACTIVE.store(TokenizerKind::Heuristic.as_u8(), Ordering::Relaxed);
        return;
    }
    tracing::debug!(tokenizer = kind.name(), "active token counter selected");
    ACTIVE.store(kind.as_u8(), Ordering::Relaxed);
}

/// The process-wide tokenizer currently in effect.
pub fn active() -> TokenizerKind {
    TokenizerKind::from_u8(ACTIVE.load(Ordering::Relaxed))
}

/// Counts tokens in `text` using the active tokenizer.
pub fn count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    match active() {
        TokenizerKind::Heuristic => heuristic(text),
        #[cfg(feature = "tokenizers")]
        TokenizerKind::O200k => real::count_o200k(text),
        #[cfg(feature = "tokenizers")]
        TokenizerKind::Cl100k => real::count_cl100k(text),
        #[cfg(not(feature = "tokenizers"))]
        _ => heuristic(text),
    }
}

/// The dependency-light estimate: ~4 bytes per token, rounded up.
fn heuristic(text: &str) -> usize {
    text.len().div_ceil(4)
}

#[cfg(feature = "tokenizers")]
mod real {
    use std::sync::OnceLock;

    use tiktoken_rs::CoreBPE;

    fn o200k() -> &'static CoreBPE {
        static BPE: OnceLock<CoreBPE> = OnceLock::new();
        BPE.get_or_init(|| tiktoken_rs::o200k_base().expect("embedded o200k_base ranks"))
    }

    fn cl100k() -> &'static CoreBPE {
        static BPE: OnceLock<CoreBPE> = OnceLock::new();
        BPE.get_or_init(|| tiktoken_rs::cl100k_base().expect("embedded cl100k_base ranks"))
    }

    pub fn count_o200k(text: &str) -> usize {
        o200k().encode_with_special_tokens(text).len()
    }

    pub fn count_cl100k(text: &str) -> usize {
        cl100k().encode_with_special_tokens(text).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_is_bytes_over_four_rounded_up() {
        assert_eq!(heuristic(""), 0);
        assert_eq!(heuristic("abcd"), 1);
        assert_eq!(heuristic("abcde"), 2);
    }

    #[test]
    fn default_active_is_heuristic_and_matches_legacy_estimate() {
        // Default process state must reproduce the historical bytes/4 estimate
        // so existing behavior and proof numbers stay byte-identical.
        assert_eq!(active(), TokenizerKind::Heuristic);
        assert_eq!(count("hello world"), "hello world".len().div_ceil(4));
        assert_eq!(count(""), 0);
    }

    #[cfg(feature = "tokenizers")]
    #[test]
    fn real_tokenizers_produce_exact_counts() {
        // "hello world" is a well-known 2-token sequence for both BPE encodings;
        // proves the feature path actually invokes the real tokenizer rather
        // than the byte heuristic (which would return 3).
        assert_eq!(real::count_o200k("hello world"), 2);
        assert_eq!(real::count_cl100k("hello world"), 2);
        assert_eq!(real::count_o200k(""), 0);
    }

    #[test]
    fn kind_names_round_trip() {
        for kind in [
            TokenizerKind::Heuristic,
            TokenizerKind::O200k,
            TokenizerKind::Cl100k,
        ] {
            assert_eq!(TokenizerKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(TokenizerKind::parse("nope"), None);
    }
}
