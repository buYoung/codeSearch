use std::env;
use std::process;

use code_search::{CodeSearchService, SearchMode, SearchRequest, SearchResults, SearchTraceResult, TraceEntry};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let command = parse_command_line(env::args().skip(1).collect())?;
    let query = command.query.clone();
    let search_results = CodeSearchService::new()
        .search_with_mode(
            SearchRequest {
                directory_path: command.directory_path,
                query,
                limit: command.limit,
            },
            command.mode,
        )
        .map_err(|error| error.to_string())?;

    print_search_results(&command.query, command.mode, &search_results);

    Ok(())
}

fn parse_command_line(arguments: Vec<String>) -> Result<SearchCommand, String> {
    if arguments.is_empty() {
        return Err(usage_message());
    }

    match arguments[0].as_str() {
        "search" => parse_search_command(&arguments[1..]),
        _ => Err(usage_message()),
    }
}

fn parse_search_command(arguments: &[String]) -> Result<SearchCommand, String> {
    if arguments.len() < 2 {
        return Err(usage_message());
    }

    let directory_path = arguments[0].clone().into();
    let mut query_parts = Vec::new();
    let mut limit = 10usize;
    let mut mode = SearchMode::Direct;
    let mut current_index = 1;

    while current_index < arguments.len() {
        match arguments[current_index].as_str() {
            "--limit" => {
                let Some(limit_value) = arguments.get(current_index + 1) else {
                    return Err("--limit requires a number".to_string());
                };
                limit = limit_value
                    .parse::<usize>()
                    .map_err(|_| "--limit must be a positive integer".to_string())?;
                current_index += 2;
            }
            "--mode" => {
                let Some(mode_value) = arguments.get(current_index + 1) else {
                    return Err("--mode requires 'direct' or 'explore'".to_string());
                };
                mode = parse_search_mode(mode_value)?;
                current_index += 2;
            }
            argument => {
                query_parts.push(argument.to_string());
                current_index += 1;
            }
        }
    }

    if query_parts.is_empty() {
        return Err("query is required".to_string());
    }

    Ok(SearchCommand {
        directory_path,
        query: query_parts.join(" "),
        limit,
        mode,
    })
}

fn parse_search_mode(value: &str) -> Result<SearchMode, String> {
    match value {
        "direct" => Ok(SearchMode::Direct),
        "explore" => Ok(SearchMode::Explore),
        _ => Err("--mode must be 'direct' or 'explore'".to_string()),
    }
}

fn print_search_results(query: &str, search_mode: SearchMode, search_results: &SearchResults) {
    println!("Scanned files: {}", search_results.scanned_file_count);
    println!("Matched targets: {}", search_results.matched_target_count);
    println!("Warnings: {}", search_results.warning_count);
    println!();

    if search_results.results.is_empty() {
        println!("No matches found.");
        return;
    }

    println!("질문: '{}'", query);
    println!("Mode: {}", search_mode);
    println!();

    match search_mode {
        SearchMode::Direct => {
            let direct_match_count = count_direct_matches(query, &search_results.results);

            if direct_match_count > 0 {
                println!("━━━ Direct matches ━━━");
                println!();
                print_result_group(
                    query,
                    search_mode,
                    &search_results.results[..direct_match_count],
                    1,
                    Some("exact"),
                );
            }

            if direct_match_count < search_results.results.len() {
                println!("━━━ Related matches ━━━");
                println!();
                print_result_group(
                    query,
                    search_mode,
                    &search_results.results[direct_match_count..],
                    direct_match_count + 1,
                    Some("related"),
                );
            }
        }
        SearchMode::Explore => {
            println!("━━━ Matches ━━━");
            println!();
            print_result_group(
                query,
                search_mode,
                &search_results.results,
                1,
                None,
            );
        }
    }
}

fn print_result_group(
    query: &str,
    search_mode: SearchMode,
    search_results: &[SearchTraceResult],
    start_index: usize,
    explicit_match_label: Option<&str>,
) {
    for (result_offset, search_result) in search_results.iter().enumerate() {
        let result_index = start_index + result_offset;
        print_result_header(
            query,
            search_mode,
            result_index,
            search_result,
            explicit_match_label,
        );
        println!();

        for section in &search_result.sections {
            println!("━━━ {} ━━━", section.title);
            for entry in &section.entries {
                print_trace_entry(entry);
            }
            println!();
        }
    }
    println!();
}

fn print_result_header(
    query: &str,
    search_mode: SearchMode,
    result_index: usize,
    search_result: &SearchTraceResult,
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

    println!("{header}");
}

fn classify_match_label(
    query: &str,
    search_mode: SearchMode,
    search_result: &SearchTraceResult,
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

fn count_direct_matches(query: &str, search_results: &[SearchTraceResult]) -> usize {
    let normalized_query = normalize_text(query);
    let has_exact_primary_match = search_results.iter().any(|search_result| {
        normalize_text(&search_result.symbol_name) == normalized_query
            && matches!(
                search_result.target_kind,
                code_search::SearchTargetKind::Function
                    | code_search::SearchTargetKind::Method
                    | code_search::SearchTargetKind::Type
            )
    });

    search_results
        .iter()
        .take_while(|search_result| {
            let is_exact_symbol_match = normalize_text(&search_result.symbol_name) == normalized_query;

            if has_exact_primary_match {
                is_exact_symbol_match
                    && matches!(
                        search_result.target_kind,
                        code_search::SearchTargetKind::Function
                            | code_search::SearchTargetKind::Method
                            | code_search::SearchTargetKind::Type
                    )
            } else {
                is_exact_direct_match(search_result, is_exact_symbol_match)
            }
        })
        .count()
}

fn is_exact_direct_match(
    search_result: &SearchTraceResult,
    is_exact_symbol_match: bool,
) -> bool {
    is_exact_symbol_match
        && matches!(
            search_result.target_kind,
            code_search::SearchTargetKind::Function
                | code_search::SearchTargetKind::Method
                | code_search::SearchTargetKind::Type
                | code_search::SearchTargetKind::LocalBinding
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

fn usage_message() -> String {
    "Usage: code-search search <directoryPath> <query> [--limit <number>] [--mode <direct|explore>]".to_string()
}

struct SearchCommand {
    directory_path: std::path::PathBuf,
    query: String,
    limit: usize,
    mode: SearchMode,
}

fn print_trace_entry(entry: &TraceEntry) {
    match entry.relationship {
        Some(relationship) => println!("  {} {}", relationship, entry.content),
        None => println!("  {}", entry.content),
    }

    if let Some(location) = &entry.location {
        println!("    → 위치: {}", location);
    }

    for annotation in &entry.annotations {
        println!("    {}", annotation);
    }
}
