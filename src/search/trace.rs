use std::collections::{HashMap, HashSet};

use crate::model::{
    SearchHit, SearchRawTarget, SearchTarget, SearchTargetKind, SectionCategory, TraceEntry,
    TraceLocation, TraceRelation, TraceSection,
};

use super::ranking::resolve_callable_index;

pub(crate) fn build_search_hit(
    target_index: usize,
    score: f64,
    search_targets: &[SearchTarget],
    callable_indices_by_name: &HashMap<String, Vec<usize>>,
    caller_index: &HashMap<usize, Vec<usize>>,
) -> SearchHit {
    let search_target = &search_targets[target_index];
    let sections = match search_target.target_kind {
        SearchTargetKind::LocalBinding => {
            build_local_binding_sections(search_target, callable_indices_by_name, search_targets)
        }
        SearchTargetKind::Function | SearchTargetKind::Method => {
            build_callable_sections(target_index, search_target, search_targets, caller_index)
        }
        SearchTargetKind::Type | SearchTargetKind::File => build_simple_sections(search_target),
    };

    SearchHit {
        score,
        target_id: search_target.target_id.clone(),
        target_kind: search_target.target_kind,
        symbol_name: search_target.symbol_name.clone(),
        file_path: search_target.file_path.clone(),
        language: search_target.language,
        line_start: search_target.line_start,
        line_end: search_target.line_end,
        sections,
        semantic_role: search_target.semantic_role.clone(),
        raw_target: SearchRawTarget {
            signature_text: search_target.signature_text.clone(),
            return_type_hint: search_target.return_type_hint.clone(),
            parameter_descriptions: search_target.parameter_descriptions.clone(),
            incoming_dependencies: search_target.incoming_dependencies.clone(),
            outgoing_dependencies: search_target.outgoing_dependencies.clone(),
            flow_steps: search_target.flow_steps.clone(),
            container_name: search_target.container_name.clone(),
            parent_symbol_name: search_target.parent_symbol_name.clone(),
            import_hint: search_target.import_hint.clone(),
        },
    }
}

fn build_local_binding_sections(
    search_target: &SearchTarget,
    callable_indices_by_name: &HashMap<String, Vec<usize>>,
    search_targets: &[SearchTarget],
) -> Vec<TraceSection> {
    let mut sections = Vec::new();
    let mut declaration_entry = TraceEntry {
        relation: None,
        text: search_target.declaration_snippet.clone(),
        location: Some(build_binding_location(search_target)),
        annotations: Vec::new(),
    };

    if let Some((type_hint, is_inferred)) =
        resolve_local_binding_type_hint(search_target, callable_indices_by_name, search_targets)
    {
        if is_inferred {
            declaration_entry
                .annotations
                .push(format!("-> Type: {type_hint} (inferred)"));
        } else {
            declaration_entry
                .annotations
                .push(format!("-> Type: {type_hint}"));
        }
    }

    sections.push(TraceSection {
        category: SectionCategory::Declaration,
        entries: vec![declaration_entry],
    });

    if !search_target.flow_steps.is_empty() {
        let mut entries = vec![TraceEntry {
            relation: None,
            text: search_target.symbol_name.clone(),
            location: None,
            annotations: Vec::new(),
        }];
        entries.extend(search_target.flow_steps.iter().map(|flow_step| TraceEntry {
            relation: Some(TraceRelation::OutgoingCall),
            text: flow_step.label.clone(),
            location: Some(build_location(
                &search_target.file_path,
                flow_step.line_start,
                flow_step.line_end,
            )),
            annotations: Vec::new(),
        }));

        sections.push(TraceSection {
            category: SectionCategory::DataFlow,
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
        relation: None,
        text: search_target
            .signature_text
            .clone()
            .unwrap_or_else(|| search_target.declaration_snippet.clone()),
        location: Some(build_location(
            &search_target.file_path,
            search_target.line_start,
            search_target.line_end,
        )),
        annotations: Vec::new(),
    };

    if let Some(return_type_hint) = &search_target.return_type_hint {
        implementation_entry
            .annotations
            .push(format!("-> Return type: {return_type_hint}"));
    }

    sections.push(TraceSection {
        category: SectionCategory::Implementation,
        entries: vec![implementation_entry],
    });

    let caller_chain_entries = build_caller_chain(target_index, search_targets, caller_index);
    if !caller_chain_entries.is_empty() {
        sections.push(TraceSection {
            category: SectionCategory::Callers,
            entries: caller_chain_entries,
        });
    }

    if let Some(dependency_section) = build_dependency_section(search_target) {
        sections.push(dependency_section);
    }

    let test_entries = collect_related_tests(target_index, search_targets, caller_index);
    if !test_entries.is_empty() {
        sections.push(TraceSection {
            category: SectionCategory::Test,
            entries: test_entries,
        });
    }

    append_enrichment_sections(&mut sections, search_target);

    sections
}

fn build_simple_sections(search_target: &SearchTarget) -> Vec<TraceSection> {
    let category = match search_target.target_kind {
        SearchTargetKind::Type => SectionCategory::Declaration,
        SearchTargetKind::File => SectionCategory::MatchCode,
        _ => SectionCategory::Declaration,
    };

    let mut sections = vec![TraceSection {
        category,
        entries: vec![TraceEntry {
            relation: None,
            text: search_target.declaration_snippet.clone(),
            location: Some(build_location(
                &search_target.file_path,
                search_target.line_start,
                search_target.line_end,
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
                relation: Some(TraceRelation::IncomingDep),
                text: parameter_description.text.clone(),
                location: None,
                annotations: Vec::new(),
            }),
    );
    entries.extend(search_target.incoming_dependencies.iter().map(|dependency| TraceEntry {
        relation: Some(TraceRelation::IncomingDep),
        text: dependency.label.clone(),
        location: None,
        annotations: Vec::new(),
    }));
    entries.extend(search_target.outgoing_dependencies.iter().map(|dependency| TraceEntry {
        relation: Some(TraceRelation::OutgoingDep),
        text: dependency.label.clone(),
        location: None,
        annotations: Vec::new(),
    }));

    if entries.is_empty() {
        return None;
    }

    Some(TraceSection {
        category: SectionCategory::Dependency,
        entries,
    })
}

fn append_enrichment_sections(sections: &mut Vec<TraceSection>, search_target: &SearchTarget) {
    if let Some(doc_comment) = &search_target.doc_comment {
        sections.push(TraceSection {
            category: SectionCategory::Documentation,
            entries: vec![TraceEntry {
                relation: None,
                text: doc_comment.clone(),
                location: None,
                annotations: Vec::new(),
            }],
        });
    }

    if !search_target.sibling_symbol_names.is_empty() {
        let sibling_list = search_target.sibling_symbol_names.join(", ");
        let text = match &search_target.container_name {
            Some(container_name) => format!("{container_name} {{ {sibling_list} }}"),
            None => sibling_list,
        };
        sections.push(TraceSection {
            category: SectionCategory::Context,
            entries: vec![TraceEntry {
                relation: None,
                text,
                location: None,
                annotations: Vec::new(),
            }],
        });
    }

    if let Some(import_hint) = &search_target.import_hint {
        sections.push(TraceSection {
            category: SectionCategory::Usage,
            entries: vec![TraceEntry {
                relation: None,
                text: import_hint.clone(),
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
        .filter(|caller_idx| search_targets[**caller_idx].semantic_role.as_deref() == Some("test"))
        .map(|caller_idx| {
            let caller_target = &search_targets[*caller_idx];
            TraceEntry {
                relation: Some(TraceRelation::OutgoingCall),
                text: format!("{}()", caller_target.symbol_name),
                location: Some(build_location(
                    &caller_target.file_path,
                    caller_target.line_start,
                    caller_target.line_end,
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
            relation: Some(TraceRelation::IncomingCall),
            text: caller_target
                .signature_text
                .clone()
                .unwrap_or_else(|| caller_target.declaration_snippet.clone()),
            location: Some(build_location(
                &caller_target.file_path,
                caller_target.line_start,
                caller_target.line_end,
            )),
            annotations: Vec::new(),
        });

        current_target_index = caller_index;
    }

    entries
}

fn build_binding_location(search_target: &SearchTarget) -> TraceLocation {
    let mut location = build_location(
        &search_target.file_path,
        search_target.line_start,
        search_target.line_end,
    );
    location.context_symbol_name = search_target.parent_symbol_name.clone();
    location
}

fn build_location(
    file_path: &std::path::Path,
    line_start: usize,
    line_end: usize,
) -> TraceLocation {
    TraceLocation::new(file_path.to_path_buf(), line_start, line_end)
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::model::{
        SearchTarget, SearchTargetKind, SupportedLanguage,
    };

    use super::build_search_hit;

    #[test]
    fn build_search_hit_uses_english_generated_annotations() {
        let search_targets = vec![build_callable_target()];
        let search_hit = build_search_hit(0, 10.0, &search_targets, &HashMap::new(), &HashMap::new());

        assert_eq!(
            search_hit.sections[0].entries[0].annotations,
            vec!["-> Return type: String".to_string()]
        );
    }

    fn build_callable_target() -> SearchTarget {
        SearchTarget {
            target_id: "src/example.rs#L10-L10:function:log".to_string(),
            file_path: PathBuf::from("src/example.rs"),
            language: SupportedLanguage::Rust,
            target_kind: SearchTargetKind::Function,
            symbol_name: "log".to_string(),
            parent_symbol_name: None,
            parent_symbol_name_search_text: None,
            line_start: 10,
            line_end: 10,
            symbol_name_search_text: "log".to_string(),
            signature_search_text: "log".to_string(),
            context_search_text: "log".to_string(),
            declaration_snippet: "fn log() -> String".to_string(),
            signature_text: Some("fn log() -> String".to_string()),
            return_type_hint: Some("String".to_string()),
            parameter_descriptions: Vec::new(),
            incoming_dependencies: Vec::new(),
            outgoing_dependencies: Vec::new(),
            flow_steps: Vec::new(),
            call_names: Vec::new(),
            doc_comment: None,
            semantic_role: None,
            sibling_symbol_names: Vec::new(),
            container_name: None,
            import_hint: None,
        }
    }
}
