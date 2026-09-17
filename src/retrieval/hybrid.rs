//! Hybrid search combining BM25 keyword score, semantic cosine similarity, and structural relevance.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::bm25::Bm25Index;
use super::embedding::{DeterministicSubwordEmbedding, EmbeddingProvider, cosine_similarity};
use crate::document::model::Document;

/// Configurable weights for hybrid score fusion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HybridWeights {
    pub bm25_weight: f32,
    pub semantic_weight: f32,
    pub structural_weight: f32,
}

impl HybridWeights {
    pub const DEFAULT: Self = Self {
        bm25_weight: 0.50,
        semantic_weight: 0.30,
        structural_weight: 0.20,
    };
}

impl Default for HybridWeights {
    fn default() -> Self {
        Self::DEFAULT
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
    /// The page this snippet's text is actually on, which is what a citation must
    /// name. `page_start` is where the unit begins, and for a multi-page section
    /// those are rarely the same page.
    pub snippet_page: u32,
    /// Index of this unit in the underlying index, so the snippet can be taken
    /// after ranking rather than for every candidate.
    #[serde(skip)]
    pub unit_index: usize,
    pub final_score: f32,
    pub bm25_score: f32,
    pub semantic_score: f32,
    pub structural_score: f32,
}

/// Why a query produced no evidence, phrased so an agent can act on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoEvidence {
    pub query: String,
    /// Query terms that appear nowhere in the corpus.
    pub absent_terms: Vec<String>,
    /// The most query information any single passage carried.
    pub best_matched_idf: f64,
    /// The information a passage needed to carry to count as evidence.
    pub required_idf: f64,
}

impl NoEvidence {
    /// Render the refusal as the agent-facing Markdown the retrieval tools emit,
    /// mirroring `EvidenceBundle::to_markdown`.
    pub fn to_markdown(&self) -> String {
        let mut out = format!("### Sin evidencia para: '{}'\n\n", self.query);
        out.push_str(
            "Ningún pasaje del corpus contiene suficientes términos informativos de la consulta.\n",
        );
        if !self.absent_terms.is_empty() {
            out.push_str(&format!(
                "\nTérminos ausentes del corpus: {}.\n",
                self.absent_terms.join(", ")
            ));
        }
        out.push_str(
            "\nSiguiente paso: usa `document_outline` para ver qué cubre el documento y en qué \
             idioma está escrito, y reformula la consulta con los términos que sí aparecen en él. \
             Consultar en un idioma distinto al del documento es motivo habitual de este aviso.\n",
        );
        out
    }
}

/// Hybrid retrieval engine holding indices and vector caches.
///
/// The weights are not part of this: an index is a function of the documents it
/// was built from, while the fusion policy is a property of the question being
/// asked. Keeping them apart is what lets one built index serve queries with
/// different weights, and what lets `RetrieverCache` key on the corpus alone.
pub struct HybridRetriever {
    bm25: Bm25Index,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    unit_embeddings: Vec<Vec<f32>>,
}

impl HybridRetriever {
    /// Build a hybrid retriever from documents with the specified or default embedding provider.
    pub fn build(docs: &[Document], provider: Option<Arc<dyn EmbeddingProvider>>) -> Self {
        let bm25 = Bm25Index::build_from_documents(docs, None);
        let provider =
            provider.unwrap_or_else(|| Arc::new(DeterministicSubwordEmbedding::default()));

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
        }
    }

    /// The lexical index behind this retriever, so a keyword-only search can
    /// reuse it instead of building a second one.
    pub fn bm25(&self) -> &Bm25Index {
        &self.bm25
    }

    /// Perform a hybrid search combining keyword matching, semantic vectors, and structural hierarchy.
    ///
    /// Admission and ranking are deliberately separate jobs. Admission ("is this
    /// evidence at all?") is decided by lexical IDF coverage, the only absolute
    /// signal here: the fused score is normalised per query, so its top hit scores
    /// the same whether or not the corpus covers the question. Ranking ("of what
    /// was admitted, what comes first?") is where the fused score belongs, and
    /// where the semantic signal is a harmless tie-breaker.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        weights: &HybridWeights,
    ) -> Result<Vec<HybridSearchHit>, NoEvidence> {
        let profile = self.bm25.profile_query(query);
        if self.bm25.units.is_empty() || profile.is_empty() {
            return Err(NoEvidence {
                query: query.to_string(),
                absent_terms: Vec::new(),
                best_matched_idf: 0.0,
                required_idf: 0.0,
            });
        }

        // 1. Admission: keep only units carrying at least the mean information of
        //    a query term. Candidates come from the postings, so this never scans
        //    the whole corpus.
        let matched = self.bm25.matched_idf(&profile);
        let admitted: Vec<usize> = matched
            .iter()
            .filter_map(|(&idx, &mass)| profile.admits(mass).then_some(idx))
            .collect();

        if admitted.is_empty() {
            return Err(NoEvidence {
                query: query.to_string(),
                absent_terms: profile
                    .absent_terms()
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                best_matched_idf: matched.values().copied().fold(0.0, f64::max),
                required_idf: profile.admission_floor(),
            });
        }

        // 2. Score every unit that matched a query term, not a window of them.
        //
        //    The window used to be (limit * 3).max(20), so an admitted unit outside
        //    it took bm25 = 0.0 and was then ranked on a character n-gram hash plus
        //    a constant. That made the result depend on how many results were asked
        //    for: the top 5 came back as a different ordering of the top 20 rather
        //    than its prefix, which no sane API does, and the bm25_score reported
        //    for explainability was simply false for those hits.
        //
        //    `matched` is the units carrying any of the query's IDF, built from the
        //    postings, so it is bounded by the query rather than the corpus and
        //    every admitted unit is in it by construction.
        let profile_terms: Vec<String> = profile.terms.iter().map(|t| t.term.clone()).collect();
        let bm25_scores = self.bm25.score_terms(&profile_terms);

        // Normalised against the best admitted unit, not the best candidate: a unit
        // that failed admission should not compress the range of the ones that did.
        let max_bm25 = admitted
            .iter()
            .filter_map(|idx| bm25_scores.get(idx).copied())
            .fold(0.0f32, f32::max)
            .max(1e-5);

        // 3. Query embedding
        let query_embedding = self.embedding_provider.embed(query).unwrap_or_default();
        let query_terms: std::collections::HashSet<&str> =
            profile.terms.iter().map(|t| t.term.as_str()).collect();

        let mut scored_hits = Vec::new();

        for idx in admitted {
            let unit = &self.bm25.units[idx];
            let bm25_raw = bm25_scores.get(&idx).copied().unwrap_or(0.0);
            let normalized_bm25 = (bm25_raw / max_bm25).clamp(0.0, 1.0);

            // Semantic score
            let semantic_score = if !query_embedding.is_empty() && idx < self.unit_embeddings.len()
            {
                cosine_similarity(&query_embedding, &self.unit_embeddings[idx])
            } else {
                0.0
            };

            // Structural score: title matches, section presence.
            //
            // Compared as terms, not substrings: `title.contains(word)` matched
            // "con" inside "Conceptos", so any Spanish particle inflated the
            // structural score of any title. Tokenizing both sides also drops the
            // stop words the raw split kept.
            let mut structural_score = 0.0f32;
            let title_terms: std::collections::HashSet<String> =
                crate::retrieval::bm25::tokenize(&unit.title)
                    .into_iter()
                    .collect();
            let matches_in_title = query_terms
                .iter()
                .filter(|t| title_terms.contains(**t))
                .count();

            if !query_terms.is_empty() {
                structural_score += (matches_in_title as f32 / query_terms.len() as f32) * 0.7;
            }

            // Bonus if unit is a dedicated section rather than a raw page. This is
            // a ranking prior only: admission already happened above, so it can no
            // longer wave every section in the corpus past the filter.
            if unit.section_id.is_some() {
                structural_score += 0.3;
            }
            let normalized_struct = structural_score.clamp(0.0, 1.0);

            // Compute weighted final score
            let final_score = (weights.bm25_weight * normalized_bm25)
                + (weights.semantic_weight * semantic_score)
                + (weights.structural_weight * normalized_struct);

            // No score threshold here: admission was decided lexically above, and
            // a cut on the fused score cannot tell relevance from noise. Measured
            // on a 437-page manual, an uncovered query scored 0.565 at the top
            // while a covered one scored 0.557 — no line separates them.
            scored_hits.push(HybridSearchHit {
                unit_id: unit.id.clone(),
                document_id: unit.document_id.clone(),
                title: unit.title.clone(),
                page_start: unit.page_start,
                page_end: unit.page_end,
                section_id: unit.section_id.clone(),
                // Filled in below, for the hits that survive the cut.
                unit_index: idx,
                snippet: String::new(),
                snippet_page: unit.page_start,
                final_score,
                bm25_score: bm25_raw,
                semantic_score,
                structural_score: normalized_struct,
            });
        }

        // Sort descending by final_score
        // Total order, not just by score. `admitted` comes out of a HashMap, whose
        // iteration order is randomised per instance, so a score-only comparator
        // left tied units in a different order on every call and `truncate` then
        // kept an arbitrary subset of the tie. Ties are common here: duplicated
        // boilerplate pages score identically on all three signals.
        scored_hits.sort_by(|a, b| {
            b.final_score
                .partial_cmp(&a.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.unit_id.cmp(&b.unit_id))
        });
        scored_hits.truncate(limit);

        // Snippets last: extracting one scans the unit's text, and doing it for
        // every candidate only to discard most of them cost about six times the
        // whole search.
        for hit in &mut scored_hits {
            let (snippet, page) = self.bm25.snippet_for(hit.unit_index, &profile_terms);
            hit.snippet = snippet;
            hit.snippet_page = page;
        }

        Ok(scored_hits)
    }
}
