pub(crate) fn tokenize_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current_token = String::new();
    let mut previous_character = None;
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        let is_alphanumeric = character.is_alphanumeric();
        if !is_alphanumeric {
            flush_token(&mut current_token, &mut tokens);
            previous_character = Some(character);
            continue;
        }

        let next_character = characters.peek().copied();

        let should_split_before_uppercase = character.is_uppercase()
            && !current_token.is_empty()
            && matches!(
                previous_character,
                Some(previous_character)
                    if previous_character.is_lowercase() || previous_character.is_numeric()
            );

        let should_split_before_acronym_tail = character.is_uppercase()
            && !current_token.is_empty()
            && matches!(
                previous_character,
                Some(previous_character) if previous_character.is_uppercase()
            )
            && matches!(next_character, Some(next_character) if next_character.is_lowercase());

        if should_split_before_uppercase || should_split_before_acronym_tail {
            flush_token(&mut current_token, &mut tokens);
        }

        current_token.extend(character.to_lowercase());
        previous_character = Some(character);
    }

    flush_token(&mut current_token, &mut tokens);

    tokens
}

pub(crate) fn condense_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn truncate_text(text: &str, max_characters: usize) -> String {
    if text.chars().count() <= max_characters {
        return text.to_string();
    }

    let mut truncated = text.chars().take(max_characters).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn flush_token(current_token: &mut String, tokens: &mut Vec<String>) {
    if current_token.is_empty() {
        return;
    }

    tokens.push(std::mem::take(current_token));
}
