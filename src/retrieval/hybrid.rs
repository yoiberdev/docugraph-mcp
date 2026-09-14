//! Hybrid search combining BM25 keyword score, semantic cosine similarity, and structural relevance.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::bm25::{Bm25Index, SearchHit};
use super::embedding::{DeterministicSubwordEmbedding, EmbeddingProvider, cosine_similarity};
use crate::document::model::Document;

/// Configurable weights for hybrid score fusion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HybridWeights {
    pub bm25_weight: f32,
    pub semantic_weight: f32,
    pub structural_weight: f32,
}

impl Default for HybridWeights {
    fn default() -> Self {
        Self {
            bm25_weight: 0.50,
            semantic_weight: 0.30,
            structural_weight: 0.20,
        }
    }
}

/// A ranked hybrid search result with score breakdown for explainability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchHit {
    pub unit_id: String,
    pub document_id: String,
    pub title: String,
    pub page_start: u32,
    pub page_end: u32,
    pub section_id: Option<String>,
    pub snippet: String,
    pub final_score: f32,
    pub bm25_score: f32,
    pub semantic_score: f32,
    pub structural_score: f32,
}

/// Hybrid retrieval engine holding indices and vector caches.
pub struct HybridRetriever {
    bm25: Bm25Index,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    unit_embeddings: Vec<Vec<f32>>,
    weights: HybridWeights,
}

impl HybridRetriever {
    /// Build a hybrid retriever from documents with the specified or default embedding provider.
    pub fn build(
        docs: &[Document],
        provider: Option<Arc<dyn EmbeddingProvider>>,
        weights: Option<HybridWeights>,
    ) -> Self {
        let bm25 = Bm25Index::build_from_documents(docs, None);
        let provider =
            provider.unwrap_or_else(|| Arc::new(DeterministicSubwordEmbedding::default()));
        let weights = weights.unwrap_or_default();

        // Precompute embeddings for search units
        let mut unit_embeddings = Vec::with_capacity(bm25.units.len());
        for unit in &bm25.units {
            let vec = provider.embed(&unit.text).unwrap_or_default();
            unit_embeddings.push(vec);
        }

        Self {
            bm25,
            embedding_provider: provider,
            unit_embeddings,
            weights,
        }
    }

    /// Perform a hybrid search combining keyword matching, semantic vectors, and structural hierarchy.
    pub fn search(&self, query: &str, limit: usize) -> Vec<HybridSearchHit> {
        if self.bm25.units.is_empty() || query.trim().is_empty() {
            return Vec::new();
        }

        // 1. BM25 search candidates (take a wider candidate set for reranking)
        let candidate_limit = limit.saturating_mul(3).max(20).min(self.bm25.units.len());
        let bm25_hits = self.bm25.search(query, candidate_limit);

        let max_bm25 = bm25_hits
            .iter()
            .map(|h| h.score)
            .fold(0.0f32, f32::max)
            .max(1e-5);

        // 2. Query embedding
        let query_embedding = self.embedding_provider.embed(query).unwrap_or_default();
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower.split_whitespace().collect();

        // Map BM25 scores by unit_id
        let bm25_map: std::collections::HashMap<&str, (f32, &SearchHit)> = bm25_hits
            .iter()
            .map(|h| (h.unit_id.as_str(), (h.score, h)))
            .collect();

        let mut scored_hits = Vec::new();

        for (idx, unit) in self.bm25.units.iter().enumerate() {
            let bm25_raw = bm25_map
                .get(unit.id.as_str())
                .map(|(s, _)| *s)
                .unwrap_or(0.0);
            let normalized_bm25 = (bm25_raw / max_bm25).clamp(0.0, 1.0);

            // Semantic score
            let semantic_score = if !query_embedding.is_empty() && idx < self.unit_embeddings.len()
            {
                cosine_similarity(&query_embedding, &self.unit_embeddings[idx])
            } else {
                0.0
            };

            // Structural score: title matches, section presence
            let mut structural_score = 0.0f32;
            let title_lower = unit.title.to_lowercase();
            let mut matches_in_title = 0;
            for w in &query_words {
                if title_lower.contains(w) {
                    matches_in_title += 1;
                }
            }

            if !query_words.is_empty() {
                structural_score += (matches_in_title as f32 / query_words.len() as f32) * 0.7;
            }

            // Bonus if unit is a dedicated section rather than a raw page
            if unit.section_id.is_some() {
                structural_score += 0.3;
            }
            let normalized_struct = structural_score.clamp(0.0, 1.0);

            // Compute weighted final score
            let final_score = (self.weights.bm25_weight * normalized_bm25)
                + (self.weights.semantic_weight * semantic_score)
                + (self.weights.structural_weight * normalized_struct);

            // Include if there is any meaningful relevance
            if final_score > 0.05 {
                let snippet = if let Some((_, hit)) = bm25_map.get(unit.id.as_str()) {
                    hit.snippet.clone()
                } else {
                    unit.text.chars().take(200).collect()
                };

                scored_hits.push(HybridSearchHit {
                    unit_id: unit.id.clone(),
                    document_id: unit.document_id.clone(),
                    title: unit.title.clone(),
                    page_start: unit.page_start,
                    page_end: unit.page_end,
                    section_id: unit.section_id.clone(),
                    snippet,
                    final_score,
                    bm25_score: bm25_raw,
                    semantic_score,
                    structural_score: normalized_struct,
                });
            }
        }

        // Sort descending by final_score
        scored_hits.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored_hits.truncate(limit);
        scored_hits
    }
}
