use crate::model::{
    SearchHit, SearchMode, SearchResults, SearchTargetKind, TraceEntry, TraceLocation,
};

pub fn render_text(query: &str, search_mode: SearchMode, search_results: &SearchResults) -> String {
    let mut output_lines = Vec::new();
    output_lines.push(format!("Scanned files: {}", search_results.scanned_file_count));
    output_lines.push(format!("Matched targets: {}", search_results.matched_target_count));
    output_lines.push(format!("Warnings: {}", search_results.warning_count));
    output_lines.push(String::new());

    if search_results.results.is_empty() {
        output_lines.push("No matches found.".to_string());
        return output_lines.join("\n");
    }

    output_lines.push(format!("질문: '{}'", query));
    output_lines.push(format!("Mode: {}", search_mode));
    output_lines.push(String::new());

    match search_mode {
        SearchMode::Direct => {
            let direct_match_count = count_direct_matches(query, &search_results.results);

            if direct_match_count > 0 {
                output_lines.push("━━━ Direct matches ━━━".to_string());
                output_lines.push(String::new());
                append_result_group(
                    &mut output_lines,
                    query,
                    search_mode,
                    &search_results.results[..direct_match_count],
                    1,
                    Some("exact"),
                );
            }

            if direct_match_count < search_results.results.len() {
                output_lines.push("━━━ Related matches ━━━".to_string());
                output_lines.push(String::new());
                append_result_group(
                    &mut output_lines,
                    query,
                    search_mode,
                    &search_results.results[direct_match_count..],
                    direct_match_count + 1,
                    Some("related"),
                );
            }
        }
        SearchMode::Explore => {
            output_lines.push("━━━ Matches ━━━".to_string());
            output_lines.push(String::new());
            append_result_group(
                &mut output_lines,
                query,
                search_mode,
                &search_results.results,
                1,
                None,
            );
        }
    }

    output_lines.join("\n")
}

fn append_result_group(
    output_lines: &mut Vec<String>,
    query: &str,
    search_mode: SearchMode,
    search_results: &[SearchHit],
    start_index: usize,
    explicit_match_label: Option<&str>,
) {
    for (result_offset, search_result) in search_results.iter().enumerate() {
        let result_index = start_index + result_offset;
        append_result_header(
            output_lines,
            query,
            search_mode,
            result_index,
            search_result,
            explicit_match_label,
        );
        output_lines.push(String::new());

        for section in &search_result.sections {
            output_lines.push(format!("━━━ {} ━━━", section.category.title()));
            for entry in &section.entries {
                append_trace_entry(output_lines, entry);
            }
            output_lines.push(String::new());
        }
    }
    output_lines.push(String::new());
}

fn append_result_header(
    output_lines: &mut Vec<String>,
    query: &str,
    search_mode: SearchMode,
    result_index: usize,
    search_result: &SearchHit,
    explicit_match_label: Option<&str>,
) {
    let match_label =
        classify_match_label(query, search_mode, search_result, explicit_match_label);
    let mut header = format!(
        "결과 {}  {}  {}  {}:{}  [{}]  score={:.3}",
        result_index,
        search_result.target_kind,
        search_result.symbol_name,
        search_result.file_path.display(),
        search_result.line_start,
        match_label,
        search_result.score
    );

    if let Some(role) = &search_result.semantic_role {
        header.push_str(&format!("  role={role}"));
    }

    output_lines.push(header);
}

fn classify_match_label(
    query: &str,
    search_mode: SearchMode,
    search_result: &SearchHit,
    explicit_match_label: Option<&str>,
) -> String {
    if let Some(match_label) = explicit_match_label {
        return match_label.to_string();
    }

    let normalized_query = normalize_text(query);
    let is_exact_symbol_match = normalize_text(&search_result.symbol_name) == normalized_query;

    match search_mode {
        SearchMode::Explore => {
            if is_exact_symbol_match {
                "exact".to_string()
            } else {
                "related".to_string()
            }
        }
        SearchMode::Direct => {
            if is_exact_direct_match(search_result, is_exact_symbol_match) {
                "exact".to_string()
            } else {
                "related".to_string()
            }
        }
    }
}

fn count_direct_matches(query: &str, search_results: &[SearchHit]) -> usize {
    let normalized_query = normalize_text(query);
    let has_exact_primary_match = search_results.iter().any(|search_result| {
        normalize_text(&search_result.symbol_name) == normalized_query
            && matches!(
                search_result.target_kind,
                SearchTargetKind::Function | SearchTargetKind::Method | SearchTargetKind::Type
            )
    });

    search_results
        .iter()
        .take_while(|search_result| {
            let is_exact_symbol_match =
                normalize_text(&search_result.symbol_name) == normalized_query;

            if has_exact_primary_match {
                is_exact_symbol_match
                    && matches!(
                        search_result.target_kind,
                        SearchTargetKind::Function
                            | SearchTargetKind::Method
                            | SearchTargetKind::Type
                    )
            } else {
                is_exact_direct_match(search_result, is_exact_symbol_match)
            }
        })
        .count()
}

fn is_exact_direct_match(search_result: &SearchHit, is_exact_symbol_match: bool) -> bool {
    is_exact_symbol_match
        && matches!(
            search_result.target_kind,
            SearchTargetKind::Function
                | SearchTargetKind::Method
                | SearchTargetKind::Type
                | SearchTargetKind::LocalBinding
        )
}

fn normalize_text(text: &str) -> String {
    text.chars()
        .map(|character| if character.is_alphanumeric() { character } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn append_trace_entry(output_lines: &mut Vec<String>, entry: &TraceEntry) {
    match entry.relation {
        Some(relation) => output_lines.push(format!("  {} {}", relation, entry.text)),
        None => output_lines.push(format!("  {}", entry.text)),
    }

    if let Some(location) = &entry.location {
        output_lines.push(format!("    → 위치: {}", format_location(location)));
    }

    for annotation in &entry.annotations {
        output_lines.push(format!("    {}", annotation));
    }
}

fn format_location(location: &TraceLocation) -> String {
    let line_text = if location.line_start == location.line_end {
        format!("{}:{}", location.file_path.display(), location.line_start)
    } else {
        format!(
            "{}:{}-{}",
            location.file_path.display(),
            location.line_start,
            location.line_end
        )
    };

    match &location.context_symbol_name {
        Some(context_symbol_name) => format!("{context_symbol_name}() @ {line_text}"),
        None => line_text,
    }
}
