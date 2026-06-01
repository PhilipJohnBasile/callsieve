use std::collections::BTreeSet;

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "for", "how", "in", "is", "of", "on", "or", "the", "to", "where",
];

pub fn tokenize(input: &str) -> Vec<String> {
    let expanded = split_camel_case(input);
    let mut tokens = BTreeSet::new();

    for token in expanded
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() > 1)
        .filter(|token| !STOP_WORDS.contains(&token.as_str()))
    {
        tokens.insert(token);
    }

    tokens.into_iter().collect()
}

fn split_camel_case(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 8);
    let mut previous_lowercase = false;

    for character in input.chars() {
        if previous_lowercase && character.is_ascii_uppercase() {
            output.push(' ');
        }
        previous_lowercase = character.is_ascii_lowercase() || character.is_ascii_digit();
        output.push(character);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_questions_and_symbols() {
        assert_eq!(
            tokenize("where is createSession handled?"),
            vec!["create", "handled", "session"]
        );
    }
}
