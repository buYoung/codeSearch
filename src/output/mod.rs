mod json;
mod response;
mod text;

use clap::ValueEnum;

use crate::model::{SearchError, SearchMode, SearchResults};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    Json,
    Text,
}

impl OutputFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Text => "text",
        }
    }
}

pub fn render_search_output(
    format: OutputFormat,
    query: &str,
    search_mode: SearchMode,
    search_results: &SearchResults,
) -> Result<String, SearchError> {
    match format {
        OutputFormat::Json => json::render_json(query, search_mode, search_results),
        OutputFormat::Text => Ok(text::render_text(query, search_mode, search_results)),
    }
}

pub fn render_runtime_error(error: &SearchError) -> Result<String, SearchError> {
    json::render_error_json(error)
}
