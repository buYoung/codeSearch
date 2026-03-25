use std::process;

use clap::{Args, Parser, Subcommand};

use code_search::{
    render_runtime_error, render_search_output, CodeSearchService, OutputFormat, SearchError,
    SearchMode, SearchRequest,
};

fn main() {
    if let Err(error) = run() {
        print_runtime_error(&error);
        process::exit(1);
    }
}

fn run() -> Result<(), SearchError> {
    let cli = Cli::parse();

    match cli.command {
        Command::Search(search_command) => run_search(search_command),
    }
}

fn run_search(search_command: SearchCommand) -> Result<(), SearchError> {
    let search_results = CodeSearchService::new().search_with_mode(
        SearchRequest {
            directory_path: search_command.directory_path,
            query: search_command.query.clone(),
            limit: search_command.limit,
        },
        search_command.mode,
    )?;
    let rendered_output = render_search_output(
        search_command.output,
        &search_command.query,
        search_command.mode,
        &search_results,
    )?;

    println!("{rendered_output}");

    Ok(())
}

fn print_runtime_error(error: &SearchError) {
    match render_runtime_error(error) {
        Ok(error_output) => println!("{error_output}"),
        Err(render_error) => println!(
            "{}",
            serde_json::json!({
                "error": {
                    "kind": render_error.kind(),
                    "message": render_error.message(),
                }
            })
        ),
    }
}

#[derive(Parser)]
#[command(name = "code-search")]
#[command(about = "Search-first code search CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Search(SearchCommand),
}

#[derive(Args)]
struct SearchCommand {
    directory_path: std::path::PathBuf,
    query: String,
    #[arg(long, default_value_t = 10)]
    limit: usize,
    #[arg(long, value_enum, default_value_t = SearchMode::Direct)]
    mode: SearchMode,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    output: OutputFormat,
}
