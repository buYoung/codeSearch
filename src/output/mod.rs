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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use crate::model::{
        SearchError, SearchHit, SearchMode, SearchRawTarget, SearchResults, SearchTargetKind,
        SectionCategory, SupportedLanguage, TraceEntry, TraceSection,
    };

    use super::{render_runtime_error, render_search_output, OutputFormat};

    #[test]
    fn render_search_output_json_preserves_english_generated_strings() {
        let rendered_output = render_search_output(
            OutputFormat::Json,
            "log",
            SearchMode::Direct,
            &build_search_results(),
        )
        .unwrap();

        assert!(!contains_hangul(&rendered_output));

        let parsed_output: Value = serde_json::from_str(&rendered_output).unwrap();
        assert_eq!(
            parsed_output["results"][0]["sections"][0]["category"],
            Value::String("implementation".to_string())
        );
        assert_eq!(
            parsed_output["results"][0]["sections"][0]["entries"][0]["annotations"][0],
            Value::String("-> Return type: ()".to_string())
        );
    }

    #[test]
    fn render_runtime_error_returns_json_with_english_message() {
        let rendered_error =
            render_runtime_error(&SearchError::InvalidRequest("limit must be greater than 0".to_string()))
                .unwrap();
        let parsed_error: Value = serde_json::from_str(&rendered_error).unwrap();

        assert_eq!(parsed_error["error"]["kind"], "invalid_request");
        assert_eq!(
            parsed_error["error"]["message"],
            "limit must be greater than 0"
        );
    }

    fn build_search_results() -> SearchResults {
        SearchResults {
            results: vec![SearchHit {
                score: 12.5,
                target_id: "src/example.rs#L10-L10:function:log".to_string(),
                target_kind: SearchTargetKind::Function,
                symbol_name: "log".to_string(),
                file_path: PathBuf::from("src/example.rs"),
                language: SupportedLanguage::Rust,
                line_start: 10,
                line_end: 10,
                sections: vec![TraceSection {
                    category: SectionCategory::Implementation,
                    entries: vec![TraceEntry {
                        relation: None,
                        text: "fn log()".to_string(),
                        location: None,
                        annotations: vec!["-> Return type: ()".to_string()],
                    }],
                }],
                semantic_role: None,
                raw_target: SearchRawTarget {
                    signature_text: Some("fn log()".to_string()),
                    return_type_hint: Some("()".to_string()),
                    parameter_descriptions: Vec::new(),
                    incoming_dependencies: Vec::new(),
                    outgoing_dependencies: Vec::new(),
                    flow_steps: Vec::new(),
                    container_name: None,
                    parent_symbol_name: None,
                    import_hint: None,
                },
            }],
            scanned_file_count: 1,
            matched_target_count: 1,
            warning_count: 0,
        }
    }

    fn contains_hangul(text: &str) -> bool {
        text.chars()
            .any(|character| ('\u{AC00}'..='\u{D7A3}').contains(&character))
    }
}
