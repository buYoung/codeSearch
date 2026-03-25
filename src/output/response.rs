use serde::Serialize;

use crate::model::{
    NamedText, SearchError, SearchHit, SearchMode, SearchResults, SearchTargetKind,
    SearchRawTarget, SectionCategory, SupportedLanguage, TraceEntry, TraceLocation, TraceReference,
    TraceRelation, TraceSection, TraceStep,
};

#[derive(Serialize)]
pub struct SearchResponse {
    pub schema_version: u32,
    pub query: String,
    pub mode: String,
    pub stats: SearchStats,
    pub results: Vec<SearchResultItem>,
}

#[derive(Serialize)]
pub struct SearchStats {
    pub scanned_file_count: usize,
    pub matched_target_count: usize,
    pub warning_count: usize,
}

#[derive(Serialize)]
pub struct SearchResultItem {
    pub score: f64,
    pub target_id: String,
    pub target_kind: String,
    pub symbol_name: String,
    pub file_path: String,
    pub language: String,
    pub line_start: usize,
    pub line_end: usize,
    pub semantic_role: Option<String>,
    pub sections: Vec<Section>,
    pub raw_target: RawTarget,
}

#[derive(Serialize)]
pub struct RawTarget {
    pub signature_text: Option<String>,
    pub return_type_hint: Option<String>,
    pub parameter_descriptions: Vec<NamedTextItem>,
    pub incoming_dependencies: Vec<TraceReferenceItem>,
    pub outgoing_dependencies: Vec<TraceReferenceItem>,
    pub flow_steps: Vec<TraceStepItem>,
    pub container_name: Option<String>,
    pub parent_symbol_name: Option<String>,
    pub import_hint: Option<String>,
}

#[derive(Serialize)]
pub struct Section {
    pub category: String,
    pub entries: Vec<SectionEntry>,
}

#[derive(Serialize)]
pub struct SectionEntry {
    pub relation: Option<String>,
    pub text: String,
    pub location: Option<LocationItem>,
    pub annotations: Vec<String>,
}

#[derive(Serialize)]
pub struct LocationItem {
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub context_symbol_name: Option<String>,
}

#[derive(Serialize)]
pub struct NamedTextItem {
    pub name: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct TraceReferenceItem {
    pub label: String,
    pub line_start: usize,
    pub line_end: usize,
    pub snippet: String,
    pub detail: Option<String>,
}

#[derive(Serialize)]
pub struct TraceStepItem {
    pub label: String,
    pub line_start: usize,
    pub line_end: usize,
    pub snippet: String,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: ErrorDetail,
}

#[derive(Serialize)]
pub struct ErrorDetail {
    pub kind: String,
    pub message: String,
}

pub fn build_search_response(
    query: &str,
    mode: SearchMode,
    search_results: &SearchResults,
) -> SearchResponse {
    SearchResponse {
        schema_version: 1,
        query: query.to_string(),
        mode: mode.as_str().to_string(),
        stats: SearchStats {
            scanned_file_count: search_results.scanned_file_count,
            matched_target_count: search_results.matched_target_count,
            warning_count: search_results.warning_count,
        },
        results: search_results
            .results
            .iter()
            .map(build_search_result_item)
            .collect(),
    }
}

pub fn build_error_response(error: &SearchError) -> ErrorResponse {
    ErrorResponse {
        error: ErrorDetail {
            kind: error.kind().to_string(),
            message: error.message(),
        },
    }
}

fn build_search_result_item(search_hit: &SearchHit) -> SearchResultItem {
    SearchResultItem {
        score: search_hit.score,
        target_id: search_hit.target_id.clone(),
        target_kind: search_target_kind_to_string(search_hit.target_kind).to_string(),
        symbol_name: search_hit.symbol_name.clone(),
        file_path: search_hit.file_path.display().to_string(),
        language: supported_language_to_string(search_hit.language).to_string(),
        line_start: search_hit.line_start,
        line_end: search_hit.line_end,
        semantic_role: search_hit.semantic_role.clone(),
        sections: search_hit.sections.iter().map(build_section).collect(),
        raw_target: build_raw_target(&search_hit.raw_target),
    }
}

fn build_raw_target(raw_target: &SearchRawTarget) -> RawTarget {
    RawTarget {
        signature_text: raw_target.signature_text.clone(),
        return_type_hint: raw_target.return_type_hint.clone(),
        parameter_descriptions: raw_target
            .parameter_descriptions
            .iter()
            .map(build_named_text_item)
            .collect(),
        incoming_dependencies: raw_target
            .incoming_dependencies
            .iter()
            .map(build_trace_reference_item)
            .collect(),
        outgoing_dependencies: raw_target
            .outgoing_dependencies
            .iter()
            .map(build_trace_reference_item)
            .collect(),
        flow_steps: raw_target
            .flow_steps
            .iter()
            .map(build_trace_step_item)
            .collect(),
        container_name: raw_target.container_name.clone(),
        parent_symbol_name: raw_target.parent_symbol_name.clone(),
        import_hint: raw_target.import_hint.clone(),
    }
}

fn build_section(section: &TraceSection) -> Section {
    Section {
        category: section_category_to_string(section.category).to_string(),
        entries: section.entries.iter().map(build_section_entry).collect(),
    }
}

fn build_section_entry(entry: &TraceEntry) -> SectionEntry {
    SectionEntry {
        relation: entry.relation.map(|relation| trace_relation_to_string(relation).to_string()),
        text: entry.text.clone(),
        location: entry.location.as_ref().map(build_location_item),
        annotations: entry.annotations.clone(),
    }
}

fn build_location_item(location: &TraceLocation) -> LocationItem {
    LocationItem {
        file_path: location.file_path.display().to_string(),
        line_start: location.line_start,
        line_end: location.line_end,
        context_symbol_name: location.context_symbol_name.clone(),
    }
}

fn build_named_text_item(named_text: &NamedText) -> NamedTextItem {
    NamedTextItem {
        name: named_text.name.clone(),
        text: named_text.text.clone(),
    }
}

fn build_trace_reference_item(trace_reference: &TraceReference) -> TraceReferenceItem {
    TraceReferenceItem {
        label: trace_reference.label.clone(),
        line_start: trace_reference.line_start,
        line_end: trace_reference.line_end,
        snippet: trace_reference.snippet.clone(),
        detail: trace_reference.detail.clone(),
    }
}

fn build_trace_step_item(trace_step: &TraceStep) -> TraceStepItem {
    TraceStepItem {
        label: trace_step.label.clone(),
        line_start: trace_step.line_start,
        line_end: trace_step.line_end,
        snippet: trace_step.snippet.clone(),
    }
}

fn search_target_kind_to_string(target_kind: SearchTargetKind) -> &'static str {
    target_kind.as_str()
}

fn supported_language_to_string(language: SupportedLanguage) -> &'static str {
    language.as_str()
}

fn section_category_to_string(category: SectionCategory) -> &'static str {
    category.as_str()
}

fn trace_relation_to_string(relation: TraceRelation) -> &'static str {
    relation.as_str()
}
