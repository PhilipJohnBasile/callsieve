#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum QueryKind {
    Identifier,
    NaturalLanguage,
}

pub const IDENTIFIER_LEXICAL_WEIGHT: f64 = 0.85;
pub const IDENTIFIER_SEMANTIC_WEIGHT: f64 = 0.15;
pub const NATURAL_LANGUAGE_LEXICAL_WEIGHT: f64 = 0.40;
pub const NATURAL_LANGUAGE_SEMANTIC_WEIGHT: f64 = 0.60;

const SYMBOL_LOOKUP_INTENT: &[&str] = &[
    "symbol",
    "symbols",
    "function",
    "functions",
    "class",
    "classes",
    "method",
    "methods",
    "struct",
    "trait",
    "enum",
    "impl",
    "definition",
    "definitions",
];

pub fn query_kind(task: &str, query_tokens: &[String]) -> QueryKind {
    if has_raw_identifier_signal(task) || has_symbol_lookup_intent(query_tokens) {
        QueryKind::Identifier
    } else {
        QueryKind::NaturalLanguage
    }
}

impl QueryKind {
    pub fn weights(self) -> (f64, f64) {
        match self {
            Self::Identifier => (IDENTIFIER_LEXICAL_WEIGHT, IDENTIFIER_SEMANTIC_WEIGHT),
            Self::NaturalLanguage => (
                NATURAL_LANGUAGE_LEXICAL_WEIGHT,
                NATURAL_LANGUAGE_SEMANTIC_WEIGHT,
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::NaturalLanguage => "natural_language",
        }
    }

    pub fn cosine_floor(self) -> f32 {
        match self {
            Self::NaturalLanguage => 0.10,
            Self::Identifier => 0.25,
        }
    }
}

fn has_symbol_lookup_intent(query_tokens: &[String]) -> bool {
    query_tokens
        .iter()
        .any(|token| SYMBOL_LOOKUP_INTENT.contains(&token.as_str()))
}

fn has_raw_identifier_signal(task: &str) -> bool {
    let words: Vec<&str> = task.split_whitespace().collect();
    for word in &words {
        let trimmed = trim_token(word);
        if trimmed.contains("::")
            || trimmed.contains("->")
            || looks_like_path(trimmed)
            || looks_like_dotted_identifier(trimmed)
            || looks_like_camel_case(trimmed)
            || looks_like_snake_case(trimmed)
            || looks_like_screaming_case(trimmed)
        {
            return true;
        }
    }

    words.len() <= 2
        && words
            .iter()
            .any(|word| looks_symbol_shaped(trim_token(word)))
}

fn trim_token(token: &str) -> &str {
    token.trim_matches(|c: char| {
        c.is_ascii_punctuation() && !matches!(c, '_' | '-' | '.' | '/' | ':' | '>')
    })
}

fn looks_symbol_shaped(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .next()
            .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && token.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn looks_like_path(token: &str) -> bool {
    token.contains('/')
        && token
            .rsplit('/')
            .next()
            .is_some_and(|name| name.contains('.') || looks_symbol_shaped(name))
}

fn looks_like_dotted_identifier(token: &str) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|part| looks_symbol_shaped(part))
}

fn looks_like_camel_case(token: &str) -> bool {
    let has_lower = token.chars().any(|c| c.is_ascii_lowercase());
    let has_upper_after_first = token.chars().skip(1).any(|c| c.is_ascii_uppercase());
    has_lower && has_upper_after_first && looks_symbol_shaped(token)
}

fn looks_like_snake_case(token: &str) -> bool {
    token.contains('_')
        && token.chars().any(|c| c.is_ascii_lowercase())
        && token.split('_').filter(|part| !part.is_empty()).count() >= 2
        && looks_symbol_shaped(token)
}

fn looks_like_screaming_case(token: &str) -> bool {
    token.contains('_')
        && token.chars().any(|c| c.is_ascii_alphabetic())
        && token
            .chars()
            .filter(|c| c.is_ascii_alphabetic())
            .all(|c| c.is_ascii_uppercase())
        && looks_symbol_shaped(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ranker;

    #[test]
    fn classifies_identifier_signals_from_raw_task() {
        let cases = [
            "Session::resolve_redirects",
            "request.url",
            "src/auth/session.ts",
            "tokenFor",
            "token_for",
            "CALLSIEVE_EMBEDDINGS",
            "symbol createSession",
        ];
        for case in cases {
            let tokens = ranker::query_tokens(case);
            assert_eq!(query_kind(case, &tokens), QueryKind::Identifier, "{case}");
        }
    }

    #[test]
    fn classifies_natural_language_tasks() {
        let task = "fix redirect handling when location header is relative";
        let tokens = ranker::query_tokens(task);
        assert_eq!(query_kind(task, &tokens), QueryKind::NaturalLanguage);
    }

    #[test]
    fn reads_raw_casing_not_only_normalized_tokens() {
        let raw = "change tokenFor behavior";
        let lower = "change token for behavior";
        let lower_tokens = ranker::query_tokens(lower);
        assert_eq!(query_kind(raw, &lower_tokens), QueryKind::Identifier);
        assert_eq!(query_kind(lower, &lower_tokens), QueryKind::NaturalLanguage);
    }
}
