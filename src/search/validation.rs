use std::path::PathBuf;

use crate::model::{SearchError, SearchRequest};
use crate::text::tokenize_text;

pub(crate) fn validate_request(request: &SearchRequest) -> Result<PathBuf, SearchError> {
    if request.limit == 0 {
        return Err(SearchError::InvalidRequest(
            "limit must be greater than 0".to_string(),
        ));
    }

    let normalized_query = tokenize_text(&request.query).join(" ");
    if normalized_query.is_empty() {
        return Err(SearchError::InvalidRequest(
            "query must include at least one searchable token".to_string(),
        ));
    }

    let canonical_directory_path = request.directory_path.canonicalize()?;
    if !canonical_directory_path.is_dir() {
        return Err(SearchError::InvalidRequest(format!(
            "directory does not exist: {}",
            request.directory_path.display()
        )));
    }

    Ok(canonical_directory_path)
}
