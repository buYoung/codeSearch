use crate::model::{SearchError, SearchMode, SearchResults};

use super::response::{build_error_response, build_search_response};

pub fn render_json(
    query: &str,
    search_mode: SearchMode,
    search_results: &SearchResults,
) -> Result<String, SearchError> {
    serde_json::to_string_pretty(&build_search_response(query, search_mode, search_results))
        .map_err(|error| SearchError::SearchEngine(error.to_string()))
}

pub fn render_error_json(error: &SearchError) -> Result<String, SearchError> {
    serde_json::to_string_pretty(&build_error_response(error))
        .map_err(|error| SearchError::SearchEngine(error.to_string()))
}
