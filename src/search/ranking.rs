use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use tantivy::collector::{Count, MultiCollector, TopDocs};
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, FAST, STORED, TEXT};
use tantivy::{DocAddress, Index, Score, doc};

use crate::model::{SearchError, SearchMode, SearchTarget, SearchTargetKind};
use crate::text::tokenize_text;

#[derive(Clone, Debug)]
pub(crate) struct ScoredSearchTarget {
    pub(crate) target_index: usize,
    pub(crate) score: f64,
    pub(crate) is_direct_match: bool,
}

pub(crate) struct RankingArtifacts {
    pub(crate) scored_targets: Vec<ScoredSearchTarget>,
    pub(crate) matched_target_count: usize,
    pub(crate) callable_indices_by_name: HashMap<String, Vec<usize>>,
    pub(crate) caller_index: HashMap<usize, Vec<usize>>,
}

pub(crate) fn rank_search_targets(
    query: &str,
    search_mode: SearchMode,
    search_targets: &[SearchTarget],
    result_limit: usize,
) -> Result<RankingArtifacts, SearchError> {
    let normalized_query = normalize_query(query);
    let callable_indices_by_name = build_callable_indices_by_name(search_targets);
    let caller_index = build_caller_index(search_targets, &callable_indices_by_name);
    let query_context = SearchQueryContext::build(&normalized_query, search_targets);
    let search_index = TantivySearchIndex::build(search_targets)?;
    let search_query_scores =
        search_index.score_chunks(&normalized_query, search_mode, result_limit, &query_context)?;
    let mut scored_targets = search_query_scores
        .ranked_chunks
        .into_iter()
        .map(|(target_index, score)| ScoredSearchTarget {
            target_index,
            score,
            is_direct_match: query_context.is_direct_match(target_index),
        })
        .collect::<Vec<_>>();

    if search_mode == SearchMode::Direct {
        scored_targets.sort_by(|left, right| compare_direct_mode(left, right, search_targets));
    } else {
        scored_targets.sort_by(|left, right| compare_explore_mode(left, right, search_targets));
    }

    Ok(RankingArtifacts {
        scored_targets,
        matched_target_count: search_query_scores.matched_target_count,
        callable_indices_by_name,
        caller_index,
    })
}

struct SearchQueryScores {
    ranked_chunks: Vec<(usize, f64)>,
    matched_target_count: usize,
}

struct SearchQueryContext {
    direct_matches: Arc<[bool]>,
    score_adjustments: Arc<[f32]>,
    target_kinds: Arc<[SearchTargetKind]>,
    path_ranks: Arc<[u64]>,
    line_starts: Arc<[u64]>,
}

impl SearchQueryContext {
    fn build(normalized_query: &str, search_targets: &[SearchTarget]) -> Self {
        let len = search_targets.len();
        let mut exact_symbol_matches = Vec::with_capacity(len);
        let mut score_adjustments = Vec::with_capacity(len);
        let mut target_kinds = Vec::with_capacity(len);
        let mut line_starts = Vec::with_capacity(len);
        let mut exact_primary_match_exists = false;
        let mut unique_paths = BTreeMap::<&Path, ()>::new();

        for search_target in search_targets {
            let is_exact = is_exact_symbol_match(normalized_query, search_target);
            exact_symbol_matches.push(is_exact);
            if is_exact && is_primary_direct_kind(search_target.target_kind) {
                exact_primary_match_exists = true;
            }
            score_adjustments.push(score_adjustment(normalized_query, search_target, is_exact));
            target_kinds.push(search_target.target_kind);
            line_starts.push(search_target.line_start as u64);
            unique_paths.entry(search_target.file_path.as_path()).or_insert(());
        }

        let path_rank_map: BTreeMap<&Path, u64> = unique_paths
            .keys()
            .enumerate()
            .map(|(rank, path)| (*path, rank as u64))
            .collect();

        let mut direct_matches = Vec::with_capacity(len);
        let mut path_ranks = Vec::with_capacity(len);

        for (target_index, search_target) in search_targets.iter().enumerate() {
            let is_direct = match search_target.target_kind {
                SearchTargetKind::Function | SearchTargetKind::Method | SearchTargetKind::Type => {
                    exact_symbol_matches[target_index]
                }
                SearchTargetKind::LocalBinding => {
                    !exact_primary_match_exists && exact_symbol_matches[target_index]
                }
                SearchTargetKind::File => false,
            };
            direct_matches.push(is_direct);
            path_ranks.push(
                *path_rank_map
                    .get(search_target.file_path.as_path())
                    .expect("path rank should exist for every search target"),
            );
        }

        Self {
            direct_matches: Arc::from(direct_matches),
            score_adjustments: Arc::from(score_adjustments),
            target_kinds: Arc::from(target_kinds),
            path_ranks: Arc::from(path_ranks),
            line_starts: Arc::from(line_starts),
        }
    }
    fn is_direct_match(&self, target_index: usize) -> bool {
        self.direct_matches[target_index]
    }
}

#[derive(Clone, Copy, Debug)]
struct DirectRankKey {
    is_direct_match: bool,
    kind_priority: usize,
    adjusted_score: Score,
    path_rank: u64,
    line_start: u64,
}

impl Ord for DirectRankKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.is_direct_match
            .cmp(&other.is_direct_match)
            .then_with(|| other.kind_priority.cmp(&self.kind_priority))
            .then_with(|| self.adjusted_score.total_cmp(&other.adjusted_score))
            .then_with(|| other.path_rank.cmp(&self.path_rank))
            .then_with(|| other.line_start.cmp(&self.line_start))
    }
}

impl PartialEq for DirectRankKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for DirectRankKey {}

impl PartialOrd for DirectRankKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug)]
struct ExploreRankKey {
    adjusted_score: Score,
    kind_priority: usize,
    path_rank: u64,
    line_start: u64,
}

impl Ord for ExploreRankKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.adjusted_score
            .total_cmp(&other.adjusted_score)
            .then_with(|| other.kind_priority.cmp(&self.kind_priority))
            .then_with(|| other.path_rank.cmp(&self.path_rank))
            .then_with(|| other.line_start.cmp(&self.line_start))
    }
}

impl PartialEq for ExploreRankKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ExploreRankKey {}

impl PartialOrd for ExploreRankKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
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

    let mut same_file_candidate_index = None;

    for candidate_index in candidates.iter().copied() {
        if search_targets[candidate_index].file_path != reference_file_path {
            continue;
        }

        if same_file_candidate_index.replace(candidate_index).is_some() {
            return None;
        }
    }

    same_file_candidate_index
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

fn score_adjustment(
    normalized_query: &str,
    search_target: &SearchTarget,
    is_exact_symbol_match: bool,
) -> f32 {
    let mut adjustment = 0.0f32;

    if is_exact_symbol_match {
        adjustment += match search_target.target_kind {
            SearchTargetKind::Function | SearchTargetKind::Method => 0.6,
            SearchTargetKind::Type => 0.5,
            SearchTargetKind::LocalBinding => 0.4,
            SearchTargetKind::File => 0.1,
        };
    }

    if search_target
        .parent_symbol_name_search_text
        .as_ref()
        .is_some_and(|text| text == normalized_query)
    {
        adjustment += 0.1;
    }

    adjustment
}

fn normalize_query(query: &str) -> String {
    tokenize_text(query).join(" ")
}

fn is_exact_symbol_match(normalized_query: &str, search_target: &SearchTarget) -> bool {
    search_target.symbol_name_search_text == normalized_query
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
    symbol_name_field: Field,
    signature_field: Field,
    context_field: Field,
}

impl TantivySearchIndex {
    fn build(search_targets: &[SearchTarget]) -> Result<Self, SearchError> {
        let mut schema_builder = Schema::builder();
        let chunk_index_field = schema_builder.add_u64_field("chunk_index", FAST | STORED);
        let symbol_name_field = schema_builder.add_text_field("symbol_name", TEXT);
        let signature_field = schema_builder.add_text_field("signature_text", TEXT);
        let context_field = schema_builder.add_text_field("context_text", TEXT);
        let schema = schema_builder.build();
        let index = Index::create_in_ram(schema);
        let mut index_writer = index.writer(50_000_000)?;

        for (chunk_index, search_target) in search_targets.iter().enumerate() {
            index_writer.add_document(doc!(
                chunk_index_field => chunk_index as u64,
                symbol_name_field => search_target.symbol_name_search_text.as_str(),
                signature_field => search_target.signature_search_text.as_str(),
                context_field => search_target.context_search_text.as_str(),
            ))?;
        }

        index_writer.commit()?;

        Ok(Self {
            index,
            symbol_name_field,
            signature_field,
            context_field,
        })
    }

    fn score_chunks(
        &self,
        normalized_query: &str,
        search_mode: SearchMode,
        result_limit: usize,
        query_context: &SearchQueryContext,
    ) -> Result<SearchQueryScores, SearchError> {
        if normalized_query.is_empty() {
            return Ok(SearchQueryScores {
                ranked_chunks: Vec::new(),
                matched_target_count: 0,
            });
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
        let parsed_query = query_parser.parse_query(normalized_query)?;
        let chunk_index_readers = searcher
            .segment_readers()
            .iter()
            .map(|segment_reader| {
                segment_reader
                    .fast_fields()
                    .u64("chunk_index")
                    .map(|column| column.first_or_default_col(0))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let search_query_scores = match search_mode {
            SearchMode::Direct => {
                self.collect_direct_scores(&searcher, &parsed_query, result_limit, query_context)?
            }
            SearchMode::Explore => {
                self.collect_explore_scores(&searcher, &parsed_query, result_limit, query_context)?
            }
        };
        let ranked_chunks = search_query_scores
            .ranked_documents
            .into_iter()
            .map(|(score, document_address)| {
                let chunk_index = chunk_index_readers[document_address.segment_ord as usize]
                    .get_val(document_address.doc_id);
                (chunk_index as usize, score)
            })
            .collect::<Vec<_>>();

        Ok(SearchQueryScores {
            ranked_chunks,
            matched_target_count: search_query_scores.matched_target_count,
        })
    }

    fn collect_direct_scores(
        &self,
        searcher: &tantivy::Searcher,
        parsed_query: &dyn tantivy::query::Query,
        result_limit: usize,
        query_context: &SearchQueryContext,
    ) -> Result<CollectedSearchScores, SearchError> {
        let direct_matches = Arc::clone(&query_context.direct_matches);
        let score_adjustments = Arc::clone(&query_context.score_adjustments);
        let target_kinds = Arc::clone(&query_context.target_kinds);
        let path_ranks = Arc::clone(&query_context.path_ranks);
        let line_starts = Arc::clone(&query_context.line_starts);
        let mut collectors = MultiCollector::new();
        let top_documents_handle = collectors.add_collector(
            TopDocs::with_limit(result_limit.max(1)).tweak_score(
                move |segment_reader: &tantivy::SegmentReader| {
                    let chunk_index_reader = segment_reader
                        .fast_fields()
                        .u64("chunk_index")
                        .expect("chunk_index fast field should exist")
                        .first_or_default_col(0);
                    let direct_matches = Arc::clone(&direct_matches);
                    let score_adjustments = Arc::clone(&score_adjustments);
                    let target_kinds = Arc::clone(&target_kinds);
                    let path_ranks = Arc::clone(&path_ranks);
                    let line_starts = Arc::clone(&line_starts);

                    move |doc, original_score| {
                        let chunk_index = chunk_index_reader.get_val(doc) as usize;
                        let is_direct_match = direct_matches[chunk_index];
                        let kind_priority = if is_direct_match {
                            direct_kind_priority(target_kinds[chunk_index])
                        } else {
                            related_kind_priority(target_kinds[chunk_index])
                        };

                        DirectRankKey {
                            is_direct_match,
                            kind_priority,
                            adjusted_score: original_score + score_adjustments[chunk_index],
                            path_rank: path_ranks[chunk_index],
                            line_start: line_starts[chunk_index],
                        }
                    }
                },
            ),
        );
        let count_handle = collectors.add_collector(Count);
        let mut multi_fruits = searcher.search(parsed_query, &collectors)?;
        let matched_target_count = count_handle.extract(&mut multi_fruits);
        let ranked_chunks = top_documents_handle
            .extract(&mut multi_fruits)
            .into_iter()
            .map(|(rank_key, document_address)| (rank_key.adjusted_score as f64, document_address))
            .collect::<Vec<_>>();

        Ok(CollectedSearchScores {
            ranked_documents: ranked_chunks,
            matched_target_count,
        })
    }

    fn collect_explore_scores(
        &self,
        searcher: &tantivy::Searcher,
        parsed_query: &dyn tantivy::query::Query,
        result_limit: usize,
        query_context: &SearchQueryContext,
    ) -> Result<CollectedSearchScores, SearchError> {
        let score_adjustments = Arc::clone(&query_context.score_adjustments);
        let target_kinds = Arc::clone(&query_context.target_kinds);
        let path_ranks = Arc::clone(&query_context.path_ranks);
        let line_starts = Arc::clone(&query_context.line_starts);
        let mut collectors = MultiCollector::new();
        let top_documents_handle = collectors.add_collector(
            TopDocs::with_limit(result_limit.max(1)).tweak_score(
                move |segment_reader: &tantivy::SegmentReader| {
                    let chunk_index_reader = segment_reader
                        .fast_fields()
                        .u64("chunk_index")
                        .expect("chunk_index fast field should exist")
                        .first_or_default_col(0);
                    let score_adjustments = Arc::clone(&score_adjustments);
                    let target_kinds = Arc::clone(&target_kinds);
                    let path_ranks = Arc::clone(&path_ranks);
                    let line_starts = Arc::clone(&line_starts);

                    move |doc, original_score| {
                        let chunk_index = chunk_index_reader.get_val(doc) as usize;

                        ExploreRankKey {
                            adjusted_score: original_score + score_adjustments[chunk_index],
                            kind_priority: related_kind_priority(target_kinds[chunk_index]),
                            path_rank: path_ranks[chunk_index],
                            line_start: line_starts[chunk_index],
                        }
                    }
                },
            ),
        );
        let count_handle = collectors.add_collector(Count);
        let mut multi_fruits = searcher.search(parsed_query, &collectors)?;
        let matched_target_count = count_handle.extract(&mut multi_fruits);
        let ranked_chunks = top_documents_handle
            .extract(&mut multi_fruits)
            .into_iter()
            .map(|(rank_key, document_address)| (rank_key.adjusted_score as f64, document_address))
            .collect::<Vec<_>>();

        Ok(CollectedSearchScores {
            ranked_documents: ranked_chunks,
            matched_target_count,
        })
    }
}

struct CollectedSearchScores {
    ranked_documents: Vec<(f64, DocAddress)>,
    matched_target_count: usize,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{
        SearchMode, SearchTarget, SearchTargetKind, SupportedLanguage,
    };

    use super::{normalize_query, rank_search_targets};

    #[test]
    fn rank_search_targets_prioritizes_exact_primary_match_in_direct_mode() {
        let search_targets = vec![
            build_search_target(SearchTargetKind::LocalBinding, "log", 30),
            build_search_target(SearchTargetKind::Function, "log", 10),
            build_search_target(SearchTargetKind::Method, "logger", 20),
        ];

        let ranking_artifacts =
            rank_search_targets("log", SearchMode::Direct, &search_targets, search_targets.len())
                .unwrap();

        assert_eq!(ranking_artifacts.scored_targets[0].target_index, 1);
        assert!(ranking_artifacts.scored_targets[0].is_direct_match);
        assert!(!ranking_artifacts.scored_targets[1].is_direct_match);
    }

    #[test]
    fn rank_search_targets_preserves_matched_target_count_with_small_limit() {
        let search_targets = vec![
            build_search_target(SearchTargetKind::Function, "log", 10),
            build_search_target(SearchTargetKind::Method, "log", 20),
            build_search_target(SearchTargetKind::LocalBinding, "log", 30),
            build_search_target(SearchTargetKind::Function, "logger", 40),
        ];

        let ranking_artifacts =
            rank_search_targets("log", SearchMode::Direct, &search_targets, 1).unwrap();

        assert_eq!(ranking_artifacts.matched_target_count, 3);
        assert_eq!(ranking_artifacts.scored_targets.len(), 1);
        assert!(ranking_artifacts.scored_targets[0].is_direct_match);
    }

    #[test]
    fn rank_search_targets_limit_matches_full_ranking_in_explore_mode() {
        let search_targets = vec![
            build_search_target_with_path(SearchTargetKind::Function, "search", "src/a.rs", 10),
            build_search_target_with_path(SearchTargetKind::Type, "search", "src/c.rs", 30),
            build_search_target_with_path(SearchTargetKind::Method, "search", "src/b.rs", 20),
            build_search_target_with_path(SearchTargetKind::LocalBinding, "search", "src/d.rs", 40),
        ];

        let full_ranking =
            rank_search_targets("search", SearchMode::Explore, &search_targets, search_targets.len())
                .unwrap();
        let limited_ranking =
            rank_search_targets("search", SearchMode::Explore, &search_targets, 2).unwrap();

        assert_eq!(limited_ranking.matched_target_count, full_ranking.matched_target_count);
        assert_eq!(
            limited_ranking
                .scored_targets
                .iter()
                .map(|scored_target| scored_target.target_index)
                .collect::<Vec<_>>(),
            full_ranking
                .scored_targets
                .iter()
                .take(2)
                .map(|scored_target| scored_target.target_index)
                .collect::<Vec<_>>()
        );
    }

    fn build_search_target(
        target_kind: SearchTargetKind,
        symbol_name: &str,
        line_start: usize,
    ) -> SearchTarget {
        build_search_target_with_path(target_kind, symbol_name, "src/example.rs", line_start)
    }

    fn build_search_target_with_path(
        target_kind: SearchTargetKind,
        symbol_name: &str,
        file_path: &str,
        line_start: usize,
    ) -> SearchTarget {
        SearchTarget {
            target_id: format!("{file_path}#L{line_start}-L{line_start}:{target_kind}:{symbol_name}"),
            file_path: PathBuf::from(file_path),
            language: SupportedLanguage::Rust,
            target_kind,
            symbol_name: symbol_name.to_string(),
            parent_symbol_name: None,
            parent_symbol_name_search_text: None,
            line_start,
            line_end: line_start,
            symbol_name_search_text: normalize_query(symbol_name),
            signature_search_text: normalize_query(symbol_name),
            context_search_text: normalize_query(symbol_name),
            declaration_snippet: symbol_name.to_string(),
            signature_text: Some(symbol_name.to_string()),
            return_type_hint: None,
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
