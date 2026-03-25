mod model;
mod output;
mod parser;
mod search;
mod text;

pub use model::{
    build_target_id, NamedText, SearchError, SearchHit, SearchMode, SearchRawTarget, SearchRequest,
    SearchResults, SearchTargetKind, SectionCategory, SupportedLanguage, TraceEntry, TraceLocation,
    TraceReference, TraceRelation, TraceSection, TraceStep,
};
pub use output::{render_runtime_error, render_search_output, OutputFormat};
pub use search::CodeSearchService;
