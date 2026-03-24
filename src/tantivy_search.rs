use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, Value, STORED, TEXT};
use tantivy::{doc, Index, TantivyDocument};

use crate::model::{SearchError, SearchTarget};
use crate::text::tokenize_text;

pub(crate) struct TantivySearchIndex {
    index: Index,
    chunk_index_field: Field,
    symbol_name_field: Field,
    signature_field: Field,
    context_field: Field,
}

impl TantivySearchIndex {
    pub(crate) fn build(search_targets: &[SearchTarget]) -> Result<Self, SearchError> {
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

    pub(crate) fn score_chunks(&self, query: &str, result_limit: usize) -> Result<Vec<(usize, f64)>, SearchError> {
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
