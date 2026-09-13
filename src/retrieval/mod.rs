//! Hybrid retrieval engine combining BM25 full-text, vector search, and graph traversal.

pub struct SearchQuery {
    pub text: String,
    pub limit: usize,
}
