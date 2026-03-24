use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub directory_path: PathBuf,
    pub query: String,
    pub limit: usize,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct SearchTarget {
    pub file_path: PathBuf,
    pub language: SupportedLanguage,
    pub target_kind: SearchTargetKind,
    pub symbol_name: String,
    pub enclosing_symbol_name: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub searchable_text: String,
    pub display_snippet: String,
    pub declaration_snippet: String,
    pub signature_text: Option<String>,
    pub return_type_hint: Option<String>,
    pub parameter_descriptions: Vec<NamedText>,
    pub incoming_dependencies: Vec<TraceReference>,
    pub outgoing_dependencies: Vec<TraceReference>,
    pub flow_steps: Vec<TraceStep>,
    pub call_names: Vec<String>,
    pub doc_comment: Option<String>,
    pub semantic_role: Option<String>,
    pub sibling_symbol_names: Vec<String>,
    pub container_name: Option<String>,
    pub import_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NamedText {
    pub name: String,
    pub text: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct TraceReference {
    pub label: String,
    pub line_start: usize,
    pub line_end: usize,
    pub snippet: String,
    pub detail: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct TraceStep {
    pub label: String,
    pub line_start: usize,
    pub line_end: usize,
    pub snippet: String,
}

#[derive(Clone, Debug)]
pub struct SearchTraceResult {
    pub score: f64,
    pub target_kind: SearchTargetKind,
    pub symbol_name: String,
    pub file_path: PathBuf,
    pub line_start: usize,
    pub sections: Vec<TraceSection>,
    pub semantic_role: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TraceSection {
    pub title: String,
    pub entries: Vec<TraceEntry>,
}

#[derive(Clone, Debug)]
pub struct TraceEntry {
    pub relationship: Option<TraceRelationship>,
    pub content: String,
    pub location: Option<String>,
    pub annotations: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceRelationship {
    Up,
    Down,
    In,
    Out,
}

impl Display for TraceRelationship {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let marker = match self {
            Self::Up => "↑",
            Self::Down => "↓",
            Self::In => "←",
            Self::Out => "→",
        };

        formatter.write_str(marker)
    }
}

#[derive(Clone, Debug)]
pub struct SearchResults {
    pub results: Vec<SearchTraceResult>,
    pub scanned_file_count: usize,
    pub matched_target_count: usize,
    pub warning_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SupportedLanguage {
    Rust,
    Go,
    TypeScript,
}

impl SupportedLanguage {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("rs") => Some(Self::Rust),
            Some("go") => Some(Self::Go),
            Some("ts") | Some("tsx") => Some(Self::TypeScript),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchTargetKind {
    LocalBinding,
    Function,
    Method,
    Type,
    File,
}

impl SearchTargetKind {
    pub fn is_callable(&self) -> bool {
        matches!(self, Self::Function | Self::Method)
    }
}

impl Display for SearchTargetKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::LocalBinding => "local",
            Self::Function => "function",
            Self::Method => "method",
            Self::Type => "type",
            Self::File => "file",
        };

        formatter.write_str(label)
    }
}

#[derive(Debug)]
pub enum SearchError {
    InvalidRequest(String),
    Io(std::io::Error),
    SearchEngine(String),
}

impl Display for SearchError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest(message) => formatter.write_str(message),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::SearchEngine(message) => write!(formatter, "search engine error: {message}"),
        }
    }
}

impl Error for SearchError {}

impl From<std::io::Error> for SearchError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<tantivy::TantivyError> for SearchError {
    fn from(error: tantivy::TantivyError) -> Self {
        Self::SearchEngine(error.to_string())
    }
}

impl From<tantivy::query::QueryParserError> for SearchError {
    fn from(error: tantivy::query::QueryParserError) -> Self {
        Self::SearchEngine(error.to_string())
    }
}
