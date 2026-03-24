pub(crate) fn tokenize_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let characters: Vec<char> = text.chars().collect();
    let mut current_token = String::new();

    for (index, character) in characters.iter().enumerate() {
        let is_alphanumeric = character.is_alphanumeric();
        if !is_alphanumeric {
            flush_token(&mut current_token, &mut tokens);
            continue;
        }

        let previous_character = if index > 0 {
            Some(characters[index - 1])
        } else {
            None
        };
        let next_character = characters.get(index + 1).copied();

        let should_split_before_uppercase = character.is_uppercase()
            && !current_token.is_empty()
            && matches!(
                previous_character,
                Some(previous) if previous.is_lowercase() || previous.is_numeric()
            );

        let should_split_before_acronym_tail = character.is_uppercase()
            && !current_token.is_empty()
            && matches!(previous_character, Some(previous) if previous.is_uppercase())
            && matches!(next_character, Some(next) if next.is_lowercase());

        if should_split_before_uppercase || should_split_before_acronym_tail {
            flush_token(&mut current_token, &mut tokens);
        }

        current_token.extend(character.to_lowercase());
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

    tokens.push(current_token.clone());
    current_token.clear();
}
