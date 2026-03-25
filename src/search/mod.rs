mod discovery;
mod ranking;
mod trace;
mod validation;

use rayon::prelude::*;

use crate::model::{SearchError, SearchMode, SearchRequest, SearchResults};
use crate::parser::analyze_file;

pub struct CodeSearchService;

impl CodeSearchService {
    pub fn new() -> Self {
        Self
    }

    pub fn search(&self, request: SearchRequest) -> Result<SearchResults, SearchError> {
        self.search_with_mode(request, SearchMode::Direct)
    }

    pub fn search_with_mode(
        &self,
        request: SearchRequest,
        search_mode: SearchMode,
    ) -> Result<SearchResults, SearchError> {
        let canonical_directory_path = validation::validate_request(&request)?;
        let file_paths = discovery::collect_supported_files(&canonical_directory_path)?;
        let scanned_file_count = file_paths.len();
        let file_analysis_results = file_paths
            .par_iter()
            .map(|file_path| analyze_file(&canonical_directory_path, file_path))
            .collect::<Vec<_>>();
        let mut warning_count = 0usize;
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

        let ranking::RankingArtifacts {
            scored_targets,
            callable_indices_by_name,
            caller_index,
        } = ranking::rank_search_targets(&request.query, search_mode, &search_targets)?;
        let matched_target_count = scored_targets.len();
        let results = scored_targets
            .into_iter()
            .take(request.limit)
            .map(|scored_target| {
                trace::build_search_hit(
                    scored_target.target_index,
                    scored_target.score,
                    &search_targets,
                    &callable_indices_by_name,
                    &caller_index,
                )
            })
            .collect();

        Ok(SearchResults {
            results,
            scanned_file_count,
            matched_target_count,
            warning_count,
        })
    }
}
