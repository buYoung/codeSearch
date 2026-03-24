use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use rayon::prelude::*;

use crate::model::{
    SearchError, SearchRequest, SearchResults, SearchTarget, SearchTargetKind, SearchTraceResult,
    SupportedLanguage, TraceEntry, TraceRelationship, TraceSection,
};
use crate::parser::analyze_file;
use crate::tantivy_search::TantivySearchIndex;
use crate::text::tokenize_text;

pub struct CodeSearchService;

impl CodeSearchService {
    pub fn new() -> Self {
        Self
    }

    pub fn search(&self, request: SearchRequest) -> Result<SearchResults, SearchError> {
        if request.limit == 0 {
            return Err(SearchError::InvalidRequest(
                "limit must be greater than 0".to_string(),
            ));
        }

        if tokenize_text(&request.query).is_empty() {
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

        let file_paths = collect_supported_files(&canonical_directory_path)?;
        let scanned_file_count = file_paths.len();
        let file_analysis_results = file_paths
            .par_iter()
            .map(|file_path| analyze_file(&canonical_directory_path, file_path))
            .collect::<Vec<_>>();
        let mut warning_count = 0;
        let mut search_targets = Vec::new();

        for file_analysis_result in file_analysis_results {
            match file_analysis_result {
                Ok(file_analysis) => {
                    warning_count += file_analysis.warning_count;
                    search_targets.extend(file_analysis.targets);
                }
                Err(_) => {
                    warning_count += 1;
                }
            }
        }

        if search_targets.is_empty() {
            return Ok(SearchResults {
                results: Vec::new(),
                scanned_file_count,
                matched_target_count: 0,
                warning_count,
            });
        }

        let callable_indices_by_name = build_callable_indices_by_name(&search_targets);
        let caller_index = build_caller_index(&search_targets, &callable_indices_by_name);
        let search_index = TantivySearchIndex::build(&search_targets)?;
        let mut scored_targets = search_index
            .score_chunks(&request.query, search_targets.len())?
            .into_iter()
            .map(|(target_index, base_score)| {
                (
                    target_index,
                    adjust_score(base_score, &request.query, &search_targets[target_index]),
                )
            })
            .collect::<Vec<_>>();

        scored_targets.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    search_targets[left.0]
                        .file_path
                        .cmp(&search_targets[right.0].file_path)
                })
                .then_with(|| search_targets[left.0].line_start.cmp(&search_targets[right.0].line_start))
        });

        let matched_target_count = scored_targets.len();
        let results = scored_targets
            .into_iter()
            .take(request.limit)
            .map(|(target_index, score)| {
                build_trace_result(
                    target_index,
                    score,
                    &search_targets,
                    &callable_indices_by_name,
                    &caller_index,
                )
            })
            .collect::<Vec<_>>();

        Ok(SearchResults {
            results,
            scanned_file_count,
            matched_target_count,
            warning_count,
        })
    }
}

fn build_trace_result(
    target_index: usize,
    score: f64,
    search_targets: &[SearchTarget],
    callable_indices_by_name: &HashMap<String, Vec<usize>>,
    caller_index: &HashMap<usize, Vec<usize>>,
) -> SearchTraceResult {
    let search_target = &search_targets[target_index];
    let sections = match search_target.target_kind {
        SearchTargetKind::LocalBinding => build_local_binding_sections(
            search_target,
            callable_indices_by_name,
            search_targets,
        ),
        SearchTargetKind::Function | SearchTargetKind::Method => {
            build_callable_sections(target_index, search_target, search_targets, caller_index)
        }
        SearchTargetKind::Type | SearchTargetKind::File => build_simple_sections(search_target),
    };

    SearchTraceResult {
        score,
        target_kind: search_target.target_kind,
        symbol_name: search_target.symbol_name.clone(),
        file_path: search_target.file_path.clone(),
        line_start: search_target.line_start,
        sections,
        semantic_role: search_target.semantic_role.clone(),
    }
}

fn build_local_binding_sections(
    search_target: &SearchTarget,
    callable_indices_by_name: &HashMap<String, Vec<usize>>,
    search_targets: &[SearchTarget],
) -> Vec<TraceSection> {
    let mut sections = Vec::new();
    let mut declaration_entry = TraceEntry {
        relationship: None,
        content: search_target.declaration_snippet.clone(),
        location: Some(build_binding_location(search_target)),
        annotations: Vec::new(),
    };

    if let Some((type_hint, is_inferred)) =
        resolve_local_binding_type_hint(search_target, callable_indices_by_name, search_targets)
    {
        if is_inferred {
            declaration_entry
                .annotations
                .push(format!("→ 타입: {type_hint}  (추론)"));
        } else {
            declaration_entry
                .annotations
                .push(format!("→ 타입: {type_hint}"));
        }
    }

    sections.push(TraceSection {
        title: "선언".to_string(),
        entries: vec![declaration_entry],
    });

    if !search_target.flow_steps.is_empty() {
        let mut entries = vec![TraceEntry {
            relationship: None,
            content: search_target.symbol_name.clone(),
            location: None,
            annotations: Vec::new(),
        }];
        entries.extend(search_target.flow_steps.iter().map(|flow_step| TraceEntry {
            relationship: Some(TraceRelationship::Down),
            content: flow_step.label.clone(),
            location: Some(format!("{}:{}", search_target.file_path.display(), flow_step.line_start)),
            annotations: Vec::new(),
        }));

        sections.push(TraceSection {
            title: "데이터 흐름".to_string(),
            entries,
        });
    }

    if let Some(dependency_section) = build_dependency_section(search_target) {
        sections.push(dependency_section);
    }

    append_enrichment_sections(&mut sections, search_target);

    sections
}

fn build_callable_sections(
    target_index: usize,
    search_target: &SearchTarget,
    search_targets: &[SearchTarget],
    caller_index: &HashMap<usize, Vec<usize>>,
) -> Vec<TraceSection> {
    let mut sections = Vec::new();
    let mut implementation_entry = TraceEntry {
        relationship: None,
        content: search_target
            .signature_text
            .clone()
            .unwrap_or_else(|| search_target.declaration_snippet.clone()),
        location: Some(format!(
            "{}:{}",
            search_target.file_path.display(),
            search_target.line_start
        )),
        annotations: Vec::new(),
    };

    if let Some(return_type_hint) = &search_target.return_type_hint {
        implementation_entry
            .annotations
            .push(format!("→ 반환 타입: {return_type_hint}"));
    }

    sections.push(TraceSection {
        title: "구현".to_string(),
        entries: vec![implementation_entry],
    });

    let caller_chain_entries = build_caller_chain(target_index, search_targets, caller_index);
    if !caller_chain_entries.is_empty() {
        sections.push(TraceSection {
            title: "상위 호출지점".to_string(),
            entries: caller_chain_entries,
        });
    }

    if let Some(dependency_section) = build_dependency_section(search_target) {
        sections.push(dependency_section);
    }

    let test_entries = collect_related_tests(target_index, search_targets, caller_index);
    if !test_entries.is_empty() {
        sections.push(TraceSection {
            title: "테스트".to_string(),
            entries: test_entries,
        });
    }

    append_enrichment_sections(&mut sections, search_target);

    sections
}

fn build_simple_sections(search_target: &SearchTarget) -> Vec<TraceSection> {
    let title = match search_target.target_kind {
        SearchTargetKind::Type => "선언",
        SearchTargetKind::File => "매칭 코드",
        _ => "선언",
    };

    let mut sections = vec![TraceSection {
        title: title.to_string(),
        entries: vec![TraceEntry {
            relationship: None,
            content: search_target.declaration_snippet.clone(),
            location: Some(format!(
                "{}:{}",
                search_target.file_path.display(),
                search_target.line_start
            )),
            annotations: Vec::new(),
        }],
    }];

    append_enrichment_sections(&mut sections, search_target);

    sections
}

fn build_dependency_section(search_target: &SearchTarget) -> Option<TraceSection> {
    let mut entries = Vec::new();

    entries.extend(
        search_target
            .parameter_descriptions
            .iter()
            .map(|parameter_description| TraceEntry {
                relationship: Some(TraceRelationship::In),
                content: parameter_description.text.clone(),
                location: None,
                annotations: Vec::new(),
            }),
    );
    entries.extend(search_target.incoming_dependencies.iter().map(|dependency| TraceEntry {
        relationship: Some(TraceRelationship::In),
        content: dependency.label.clone(),
        location: None,
        annotations: Vec::new(),
    }));
    entries.extend(search_target.outgoing_dependencies.iter().map(|dependency| TraceEntry {
        relationship: Some(TraceRelationship::Out),
        content: dependency.label.clone(),
        location: None,
        annotations: Vec::new(),
    }));

    if entries.is_empty() {
        return None;
    }

    Some(TraceSection {
        title: "의존성".to_string(),
        entries,
    })
}

fn append_enrichment_sections(
    sections: &mut Vec<TraceSection>,
    search_target: &SearchTarget,
) {
    // Doc comment section
    if let Some(doc_comment) = &search_target.doc_comment {
        sections.push(TraceSection {
            title: "문서".to_string(),
            entries: vec![TraceEntry {
                relationship: None,
                content: doc_comment.clone(),
                location: None,
                annotations: Vec::new(),
            }],
        });
    }

    // Sibling context section
    if !search_target.sibling_symbol_names.is_empty() {
        let sibling_list = search_target.sibling_symbol_names.join(", ");
        let content = match &search_target.container_name {
            Some(container_name) => {
                let keyword = match search_target.language {
                    SupportedLanguage::Rust => "impl",
                    SupportedLanguage::TypeScript => "class",
                    SupportedLanguage::Go => "type",
                };
                format!("{keyword} {container_name} {{ {sibling_list} }}")
            }
            None => sibling_list,
        };
        sections.push(TraceSection {
            title: "주변 코드".to_string(),
            entries: vec![TraceEntry {
                relationship: None,
                content,
                location: None,
                annotations: Vec::new(),
            }],
        });
    }

    // Import hint section
    if let Some(import_hint) = &search_target.import_hint {
        sections.push(TraceSection {
            title: "사용법".to_string(),
            entries: vec![TraceEntry {
                relationship: None,
                content: import_hint.clone(),
                location: None,
                annotations: Vec::new(),
            }],
        });
    }
}

fn collect_related_tests(
    target_index: usize,
    search_targets: &[SearchTarget],
    caller_index: &HashMap<usize, Vec<usize>>,
) -> Vec<TraceEntry> {
    let Some(caller_candidates) = caller_index.get(&target_index) else {
        return Vec::new();
    };

    caller_candidates
        .iter()
        .filter(|caller_idx| {
            search_targets[**caller_idx]
                .semantic_role
                .as_deref()
                == Some("test")
        })
        .map(|caller_idx| {
            let caller_target = &search_targets[*caller_idx];
            TraceEntry {
                relationship: Some(TraceRelationship::Down),
                content: format!("{}()", caller_target.symbol_name),
                location: Some(format!(
                    "{}:{}",
                    caller_target.file_path.display(),
                    caller_target.line_start
                )),
                annotations: Vec::new(),
            }
        })
        .collect()
}

fn build_caller_chain(
    target_index: usize,
    search_targets: &[SearchTarget],
    caller_index: &HashMap<usize, Vec<usize>>,
) -> Vec<TraceEntry> {
    let mut entries = Vec::new();
    let mut visited_indices = HashSet::new();
    let mut current_target_index = target_index;

    for _ in 0..3 {
        let Some(caller_candidates) = caller_index.get(&current_target_index) else {
            break;
        };
        let Some(caller_index) =
            select_primary_caller(caller_candidates, &search_targets[current_target_index], search_targets)
        else {
            break;
        };
        if !visited_indices.insert(caller_index) {
            break;
        }

        let caller_target = &search_targets[caller_index];
        entries.push(TraceEntry {
            relationship: Some(TraceRelationship::Up),
            content: caller_target
                .signature_text
                .clone()
                .unwrap_or_else(|| caller_target.declaration_snippet.clone()),
            location: Some(format!(
                "{}:{}",
                caller_target.file_path.display(),
                caller_target.line_start
            )),
            annotations: Vec::new(),
        });

        current_target_index = caller_index;
    }

    entries
}

fn build_binding_location(search_target: &SearchTarget) -> String {
    match &search_target.enclosing_symbol_name {
        Some(enclosing_symbol_name) => format!(
            "{}() @ {}:{}",
            enclosing_symbol_name,
            search_target.file_path.display(),
            search_target.line_start
        ),
        None => format!("{}:{}", search_target.file_path.display(), search_target.line_start),
    }
}

fn resolve_local_binding_type_hint(
    search_target: &SearchTarget,
    callable_indices_by_name: &HashMap<String, Vec<usize>>,
    search_targets: &[SearchTarget],
) -> Option<(String, bool)> {
    if let Some(type_hint) = &search_target.return_type_hint {
        return Some((type_hint.clone(), false));
    }

    if search_target.call_names.len() == 1 {
        let call_name = &search_target.call_names[0];
        let Some(callable_index) = resolve_callable_index(
            call_name,
            &search_target.file_path,
            callable_indices_by_name,
            search_targets,
        ) else {
            return None;
        };
        let Some(return_type_hint) = &search_targets[callable_index].return_type_hint else {
            return None;
        };

        return Some((return_type_hint.clone(), true));
    }

    None
}

fn build_callable_indices_by_name(
    search_targets: &[SearchTarget],
) -> HashMap<String, Vec<usize>> {
    let mut callable_indices_by_name = HashMap::<String, Vec<usize>>::new();

    for (target_index, search_target) in search_targets.iter().enumerate() {
        if !search_target.target_kind.is_callable() {
            continue;
        }

        callable_indices_by_name
            .entry(search_target.symbol_name.clone())
            .or_default()
            .push(target_index);
    }

    callable_indices_by_name
}

fn build_caller_index(
    search_targets: &[SearchTarget],
    callable_indices_by_name: &HashMap<String, Vec<usize>>,
) -> HashMap<usize, Vec<usize>> {
    let mut caller_index = HashMap::<usize, Vec<usize>>::new();

    for (caller_target_index, caller_target) in search_targets.iter().enumerate() {
        if !caller_target.target_kind.is_callable() {
            continue;
        }

        for call_name in &caller_target.call_names {
            let Some(callee_target_index) = resolve_callable_index(
                call_name,
                &caller_target.file_path,
                callable_indices_by_name,
                search_targets,
            ) else {
                continue;
            };
            if callee_target_index == caller_target_index {
                continue;
            }

            caller_index
                .entry(callee_target_index)
                .or_default()
                .push(caller_target_index);
        }
    }

    for caller_candidates in caller_index.values_mut() {
        caller_candidates.sort_by(|left, right| {
            search_targets[*left]
                .file_path
                .cmp(&search_targets[*right].file_path)
                .then_with(|| search_targets[*left].line_start.cmp(&search_targets[*right].line_start))
        });
        caller_candidates.dedup();
    }

    caller_index
}

fn resolve_callable_index(
    call_name: &str,
    reference_file_path: &Path,
    callable_indices_by_name: &HashMap<String, Vec<usize>>,
    search_targets: &[SearchTarget],
) -> Option<usize> {
    let candidates = callable_indices_by_name.get(call_name)?;
    if candidates.len() == 1 {
        return candidates.first().copied();
    }

    let same_file_candidates = candidates
        .iter()
        .copied()
        .filter(|candidate_index| search_targets[*candidate_index].file_path == reference_file_path)
        .collect::<Vec<_>>();
    if same_file_candidates.len() == 1 {
        return same_file_candidates.first().copied();
    }

    None
}

fn select_primary_caller(
    caller_candidates: &[usize],
    current_target: &SearchTarget,
    search_targets: &[SearchTarget],
) -> Option<usize> {
    let mut sorted_candidates = caller_candidates.to_vec();
    sorted_candidates.sort_by(|left, right| {
        let left_target = &search_targets[*left];
        let right_target = &search_targets[*right];
        let left_same_file = left_target.file_path == current_target.file_path;
        let right_same_file = right_target.file_path == current_target.file_path;

        right_same_file
            .cmp(&left_same_file)
            .then_with(|| left_target.file_path.cmp(&right_target.file_path))
            .then_with(|| left_target.line_start.cmp(&right_target.line_start))
    });

    sorted_candidates.into_iter().next()
}

fn adjust_score(
    base_score: f64,
    query: &str,
    search_target: &SearchTarget,
) -> f64 {
    let normalized_query = tokenize_text(query).join(" ");
    let normalized_symbol_name = tokenize_text(&search_target.symbol_name).join(" ");
    let mut adjusted_score = base_score;

    if normalized_query == normalized_symbol_name {
        adjusted_score += match search_target.target_kind {
            SearchTargetKind::LocalBinding => 8.0,
            SearchTargetKind::Function | SearchTargetKind::Method => 6.0,
            SearchTargetKind::Type => 5.0,
            SearchTargetKind::File => 2.0,
        };
    }

    if search_target
        .enclosing_symbol_name
        .as_ref()
        .map(|name| tokenize_text(name).join(" "))
        .is_some_and(|enclosing_name| enclosing_name == normalized_query)
    {
        adjusted_score += 1.0;
    }

    adjusted_score
}

fn collect_supported_files(directory_path: &Path) -> Result<Vec<PathBuf>, SearchError> {
    let mut file_paths = Vec::new();
    let walker = WalkBuilder::new(directory_path)
        .standard_filters(true)
        .build();

    for entry in walker {
        let directory_entry = match entry {
            Ok(directory_entry) => directory_entry,
            Err(_) => continue,
        };

        let Some(file_type) = directory_entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        if SupportedLanguage::from_path(directory_entry.path()).is_some() {
            file_paths.push(directory_entry.into_path());
        }
    }

    file_paths.sort();
    Ok(file_paths)
}
