use std::env;
use std::process;

use code_search::{CodeSearchService, SearchRequest, SearchResults, TraceEntry};

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
        .search(SearchRequest {
            directory_path: command.directory_path,
            query,
            limit: command.limit,
        })
        .map_err(|error| error.to_string())?;

    print_search_results(&command.query, &search_results);

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
    })
}

fn print_search_results(query: &str, search_results: &SearchResults) {
    println!("Scanned files: {}", search_results.scanned_file_count);
    println!("Matched targets: {}", search_results.matched_target_count);
    println!("Warnings: {}", search_results.warning_count);
    println!();

    if search_results.results.is_empty() {
        println!("No matches found.");
        return;
    }

    println!("질문: '{}'", query);
    println!();

    for (result_index, search_result) in search_results.results.iter().enumerate() {
        match &search_result.semantic_role {
            Some(role) => println!("결과 {}  score={:.3}  [{}]", result_index + 1, search_result.score, role),
            None => println!("결과 {}  score={:.3}", result_index + 1, search_result.score),
        }
        println!();

        for section in &search_result.sections {
            println!("━━━ {} ━━━", section.title);
            for entry in &section.entries {
                print_trace_entry(entry);
            }
            println!();
        }
    }
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

fn usage_message() -> String {
    "Usage: code-search search <directoryPath> <query> [--limit <number>]".to_string()
}

struct SearchCommand {
    directory_path: std::path::PathBuf,
    query: String,
    limit: usize,
}
