use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, Value, STORED, TEXT};
use tantivy::{doc, Index, TantivyDocument};

use crate::model::{SearchError, SearchTarget};
use crate::text::tokenize_text;

pub(crate) struct TantivySearchIndex {
    index: Index,
    chunk_index_field: Field,
    searchable_text_field: Field,
}

impl TantivySearchIndex {
    pub(crate) fn build(search_targets: &[SearchTarget]) -> Result<Self, SearchError> {
        let mut schema_builder = Schema::builder();
        let chunk_index_field = schema_builder.add_u64_field("chunk_index", STORED);
        let searchable_text_field = schema_builder.add_text_field("searchable_text", TEXT);
        let schema = schema_builder.build();
        let index = Index::create_in_ram(schema);
        let mut index_writer = index.writer(50_000_000)?;

        for (chunk_index, search_target) in search_targets.iter().enumerate() {
            index_writer.add_document(doc!(
                chunk_index_field => chunk_index as u64,
                searchable_text_field => search_target.searchable_text.clone(),
            ))?;
        }

        index_writer.commit()?;

        Ok(Self {
            index,
            chunk_index_field,
            searchable_text_field,
        })
    }

    pub(crate) fn score_chunks(&self, query: &str, result_limit: usize) -> Result<Vec<(usize, f64)>, SearchError> {
        let normalized_query = tokenize_text(query).join(" ");
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }

        let reader = self.index.reader()?;
        let searcher = reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.searchable_text_field]);
        let parsed_query = query_parser.parse_query(&normalized_query)?;
        let top_documents = searcher.search(&parsed_query, &TopDocs::with_limit(result_limit.max(1)))?;
        let mut scored_chunks = Vec::new();

        for (score, document_address) in top_documents {
            let document: TantivyDocument = searcher.doc(document_address)?;
            let Some(chunk_index) = document
                .get_first(self.chunk_index_field)
                .and_then(|value| value.as_u64()) else {
                continue;
            };

            scored_chunks.push((chunk_index as usize, score as f64));
        }

        Ok(scored_chunks)
    }
}
