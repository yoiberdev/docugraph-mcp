//! Hybrid retrieval engine combining BM25 full-text, vector search, and graph traversal.

pub mod bm25;
pub mod cache;
pub mod context;
pub mod embedding;
pub mod hybrid;

pub use bm25::{Bm25Config, Bm25Index, QueryProfile, SearchHit};
pub use cache::{CorpusSignature, RetrieverCache};
pub use context::{ContextBudget, ContextBuilder, EvidenceBundle, EvidenceItem, estimate_tokens};
pub use embedding::{DeterministicSubwordEmbedding, EmbeddingProvider};
pub use hybrid::{HybridRetriever, HybridSearchHit, HybridWeights, NoEvidence};
