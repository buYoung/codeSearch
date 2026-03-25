use clap::ValueEnum;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SearchRequest {
    pub directory_path: PathBuf,
    pub query: String,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum SearchMode {
    Direct,
    Explore,
}

impl SearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Explore => "explore",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchTarget {
    pub target_id: String,
    pub file_path: PathBuf,
    pub language: SupportedLanguage,
    pub target_kind: SearchTargetKind,
    pub symbol_name: String,
    pub parent_symbol_name: Option<String>,
    pub parent_symbol_name_search_text: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub symbol_name_search_text: String,
    pub signature_search_text: String,
    pub context_search_text: String,
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

#[derive(Clone, Debug)]
pub struct TraceReference {
    pub label: String,
    pub line_start: usize,
    pub line_end: usize,
    pub snippet: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TraceStep {
    pub label: String,
    pub line_start: usize,
    pub line_end: usize,
    pub snippet: String,
}

#[derive(Clone, Debug)]
pub struct SearchRawTarget {
    pub signature_text: Option<String>,
    pub return_type_hint: Option<String>,
    pub parameter_descriptions: Vec<NamedText>,
    pub incoming_dependencies: Vec<TraceReference>,
    pub outgoing_dependencies: Vec<TraceReference>,
    pub flow_steps: Vec<TraceStep>,
    pub container_name: Option<String>,
    pub parent_symbol_name: Option<String>,
    pub import_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub score: f64,
    pub target_id: String,
    pub target_kind: SearchTargetKind,
    pub symbol_name: String,
    pub file_path: PathBuf,
    pub language: SupportedLanguage,
    pub line_start: usize,
    pub line_end: usize,
    pub sections: Vec<TraceSection>,
    pub semantic_role: Option<String>,
    pub raw_target: SearchRawTarget,
}

#[derive(Clone, Debug)]
pub struct TraceSection {
    pub category: SectionCategory,
    pub entries: Vec<TraceEntry>,
}

#[derive(Clone, Debug)]
pub struct TraceEntry {
    pub relation: Option<TraceRelation>,
    pub text: String,
    pub location: Option<TraceLocation>,
    pub annotations: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TraceLocation {
    pub file_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub context_symbol_name: Option<String>,
}

impl TraceLocation {
    pub fn new(file_path: PathBuf, line_start: usize, line_end: usize) -> Self {
        Self {
            file_path,
            line_start,
            line_end,
            context_symbol_name: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceRelation {
    IncomingCall,
    OutgoingCall,
    IncomingDep,
    OutgoingDep,
}

impl TraceRelation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::IncomingCall => "incoming_call",
            Self::OutgoingCall => "outgoing_call",
            Self::IncomingDep => "incoming_dep",
            Self::OutgoingDep => "outgoing_dep",
        }
    }
}

impl Display for TraceRelation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let marker = match self {
            Self::IncomingCall => "↑",
            Self::OutgoingCall => "↓",
            Self::IncomingDep => "←",
            Self::OutgoingDep => "→",
        };

        formatter.write_str(marker) 
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectionCategory {
    Declaration,
    DataFlow,
    Dependency,
    Implementation,
    Callers,
    Test,
    Documentation,
    Context,
    Usage,
    MatchCode,
}

impl SectionCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::DataFlow => "data_flow",
            Self::Dependency => "dependency",
            Self::Implementation => "implementation",
            Self::Callers => "callers",
            Self::Test => "test",
            Self::Documentation => "documentation",
            Self::Context => "context",
            Self::Usage => "usage",
            Self::MatchCode => "match_code",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Declaration => "Declaration",
            Self::DataFlow => "Data Flow",
            Self::Dependency => "Dependencies",
            Self::Implementation => "Implementation",
            Self::Callers => "Callers",
            Self::Test => "Tests",
            Self::Documentation => "Documentation",
            Self::Context => "Context",
            Self::Usage => "Usage",
            Self::MatchCode => "Matching Code",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchResults {
    pub results: Vec<SearchHit>,
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

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::TypeScript => "typescript",
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

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LocalBinding => "local_binding",
            Self::Function => "function",
            Self::Method => "method",
            Self::Type => "type",
            Self::File => "file",
        }
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

impl Display for SearchMode {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn build_target_id(
    file_path: &Path,
    line_start: usize,
    line_end: usize,
    target_kind: SearchTargetKind,
    symbol_name: &str,
) -> String {
    format!(
        "{}#L{}-L{}:{}:{}",
        file_path.display(),
        line_start,
        line_end,
        target_kind.as_str(),
        symbol_name
    )
}

#[derive(Debug)]
pub enum SearchError {
    InvalidRequest(String),
    Io(std::io::Error),
    SearchEngine(String),
}

impl SearchError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::Io(_) => "io",
            Self::SearchEngine(_) => "search_engine",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidRequest(message) => message.clone(),
            Self::Io(error) => format!("I/O error: {error}"),
            Self::SearchEngine(message) => format!("search engine error: {message}"),
        }
    }
}

impl Display for SearchError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
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
