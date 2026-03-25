use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, Value, STORED, TEXT};
use tantivy::{doc, Index, TantivyDocument};

use crate::model::{SearchError, SearchMode, SearchTarget, SearchTargetKind};
use crate::text::tokenize_text;

#[derive(Clone, Debug)]
pub(crate) struct ScoredSearchTarget {
    pub(crate) target_index: usize,
    pub(crate) score: f64,
    pub(crate) is_exact_symbol_match: bool,
    pub(crate) is_direct_match: bool,
}

pub(crate) struct RankingArtifacts {
    pub(crate) scored_targets: Vec<ScoredSearchTarget>,
    pub(crate) callable_indices_by_name: HashMap<String, Vec<usize>>,
    pub(crate) caller_index: HashMap<usize, Vec<usize>>,
}

pub(crate) fn rank_search_targets(
    query: &str,
    search_mode: SearchMode,
    search_targets: &[SearchTarget],
) -> Result<RankingArtifacts, SearchError> {
    let normalized_query = normalize_query(query);
    let callable_indices_by_name = build_callable_indices_by_name(search_targets);
    let caller_index = build_caller_index(search_targets, &callable_indices_by_name);
    let search_index = TantivySearchIndex::build(search_targets)?;
    let mut scored_targets = search_index
        .score_chunks(query, search_targets.len())?
        .into_iter()
        .map(|(target_index, base_score)| {
            let search_target = &search_targets[target_index];
            let is_exact_symbol_match = is_exact_symbol_match(&normalized_query, search_target);

            ScoredSearchTarget {
                target_index,
                score: adjust_score(base_score, &normalized_query, search_target),
                is_exact_symbol_match,
                is_direct_match: false,
            }
        })
        .collect::<Vec<_>>();

    let exact_primary_match_exists = scored_targets.iter().any(|scored_target| {
        scored_target.is_exact_symbol_match
            && is_primary_direct_kind(search_targets[scored_target.target_index].target_kind)
    });

    if search_mode == SearchMode::Direct {
        for scored_target in &mut scored_targets {
            let target_kind = search_targets[scored_target.target_index].target_kind;
            scored_target.is_direct_match = match target_kind {
                SearchTargetKind::Function | SearchTargetKind::Method | SearchTargetKind::Type => {
                    scored_target.is_exact_symbol_match
                }
                SearchTargetKind::LocalBinding => {
                    !exact_primary_match_exists && scored_target.is_exact_symbol_match
                }
                SearchTargetKind::File => false,
            };
        }

        scored_targets.sort_by(|left, right| compare_direct_mode(left, right, search_targets));
    } else {
        scored_targets.sort_by(|left, right| compare_explore_mode(left, right, search_targets));
    }

    Ok(RankingArtifacts {
        scored_targets,
        callable_indices_by_name,
        caller_index,
    })
}

pub(crate) fn resolve_callable_index(
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

fn build_callable_indices_by_name(search_targets: &[SearchTarget]) -> HashMap<String, Vec<usize>> {
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

fn adjust_score(base_score: f64, normalized_query: &str, search_target: &SearchTarget) -> f64 {
    let normalized_symbol_name = tokenize_text(&search_target.symbol_name).join(" ");
    let mut adjusted_score = base_score;

    if normalized_query == normalized_symbol_name {
        adjusted_score += match search_target.target_kind {
            SearchTargetKind::Function | SearchTargetKind::Method => 0.6,
            SearchTargetKind::Type => 0.5,
            SearchTargetKind::LocalBinding => 0.4,
            SearchTargetKind::File => 0.1,
        };
    }

    if search_target
        .parent_symbol_name
        .as_ref()
        .map(|name| tokenize_text(name).join(" "))
        .is_some_and(|parent_name| parent_name == normalized_query)
    {
        adjusted_score += 0.1;
    }

    adjusted_score
}

fn normalize_query(query: &str) -> String {
    tokenize_text(query).join(" ")
}

fn is_exact_symbol_match(normalized_query: &str, search_target: &SearchTarget) -> bool {
    tokenize_text(&search_target.symbol_name).join(" ") == normalized_query
}

fn is_primary_direct_kind(target_kind: SearchTargetKind) -> bool {
    matches!(
        target_kind,
        SearchTargetKind::Function | SearchTargetKind::Method | SearchTargetKind::Type
    )
}

fn compare_direct_mode(
    left: &ScoredSearchTarget,
    right: &ScoredSearchTarget,
    search_targets: &[SearchTarget],
) -> Ordering {
    right
        .is_direct_match
        .cmp(&left.is_direct_match)
        .then_with(|| {
            let left_kind_priority = if left.is_direct_match {
                direct_kind_priority(search_targets[left.target_index].target_kind)
            } else {
                related_kind_priority(search_targets[left.target_index].target_kind)
            };
            let right_kind_priority = if right.is_direct_match {
                direct_kind_priority(search_targets[right.target_index].target_kind)
            } else {
                related_kind_priority(search_targets[right.target_index].target_kind)
            };

            left_kind_priority.cmp(&right_kind_priority)
        })
        .then_with(|| right.score.partial_cmp(&left.score).unwrap_or(Ordering::Equal))
        .then_with(|| {
            search_targets[left.target_index]
                .file_path
                .cmp(&search_targets[right.target_index].file_path)
        })
        .then_with(|| {
            search_targets[left.target_index]
                .line_start
                .cmp(&search_targets[right.target_index].line_start)
        })
}

fn compare_explore_mode(
    left: &ScoredSearchTarget,
    right: &ScoredSearchTarget,
    search_targets: &[SearchTarget],
) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            related_kind_priority(search_targets[left.target_index].target_kind)
                .cmp(&related_kind_priority(search_targets[right.target_index].target_kind))
        })
        .then_with(|| {
            search_targets[left.target_index]
                .file_path
                .cmp(&search_targets[right.target_index].file_path)
        })
        .then_with(|| {
            search_targets[left.target_index]
                .line_start
                .cmp(&search_targets[right.target_index].line_start)
        })
}

fn direct_kind_priority(target_kind: SearchTargetKind) -> usize {
    match target_kind {
        SearchTargetKind::Function | SearchTargetKind::Method => 0,
        SearchTargetKind::Type => 1,
        SearchTargetKind::LocalBinding => 2,
        SearchTargetKind::File => 3,
    }
}

fn related_kind_priority(target_kind: SearchTargetKind) -> usize {
    match target_kind {
        SearchTargetKind::Function | SearchTargetKind::Method => 0,
        SearchTargetKind::Type => 1,
        SearchTargetKind::LocalBinding => 2,
        SearchTargetKind::File => 3,
    }
}

struct TantivySearchIndex {
    index: Index,
    chunk_index_field: Field,
    symbol_name_field: Field,
    signature_field: Field,
    context_field: Field,
}

impl TantivySearchIndex {
    fn build(search_targets: &[SearchTarget]) -> Result<Self, SearchError> {
        let mut schema_builder = Schema::builder();
        let chunk_index_field = schema_builder.add_u64_field("chunk_index", STORED);
        let symbol_name_field = schema_builder.add_text_field("symbol_name", TEXT);
        let signature_field = schema_builder.add_text_field("signature_text", TEXT);
        let context_field = schema_builder.add_text_field("context_text", TEXT);
        let schema = schema_builder.build();
        let index = Index::create_in_ram(schema);
        let mut index_writer = index.writer(50_000_000)?;

        for (chunk_index, search_target) in search_targets.iter().enumerate() {
            index_writer.add_document(doc!(
                chunk_index_field => chunk_index as u64,
                symbol_name_field => search_target.symbol_name_search_text.clone(),
                signature_field => search_target.signature_search_text.clone(),
                context_field => search_target.context_search_text.clone(),
            ))?;
        }

        index_writer.commit()?;

        Ok(Self {
            index,
            chunk_index_field,
            symbol_name_field,
            signature_field,
            context_field,
        })
    }

    fn score_chunks(&self, query: &str, result_limit: usize) -> Result<Vec<(usize, f64)>, SearchError> {
        let normalized_query = tokenize_text(query).join(" ");
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }

        let reader = self.index.reader()?;
        let searcher = reader.searcher();
        let mut query_parser = QueryParser::for_index(
            &self.index,
            vec![self.symbol_name_field, self.signature_field, self.context_field],
        );
        query_parser.set_field_boost(self.symbol_name_field, 6.0);
        query_parser.set_field_boost(self.signature_field, 3.0);
        query_parser.set_field_boost(self.context_field, 1.0);
        let parsed_query = query_parser.parse_query(&normalized_query)?;
        let top_documents =
            searcher.search(&parsed_query, &TopDocs::with_limit(result_limit.max(1)))?;
        let mut scored_chunks = Vec::new();

        for (score, document_address) in top_documents {
            let document: TantivyDocument = searcher.doc(document_address)?;
            let Some(chunk_index) = document
                .get_first(self.chunk_index_field)
                .and_then(|value| value.as_u64())
            else {
                continue;
            };

            scored_chunks.push((chunk_index as usize, score as f64));
        }

        Ok(scored_chunks)
    }
}
