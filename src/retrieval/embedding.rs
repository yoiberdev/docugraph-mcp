//! Modular embedding provider abstractions and vector operations.

use anyhow::Result;
use sha2::{Digest, Sha256};

/// Trait for generating semantic or lexical embedding vectors.
pub trait EmbeddingProvider: Send + Sync {
    /// Dimension of the output vectors.
    fn dimension(&self) -> usize;

    /// Embed a piece of text into a normalized vector.
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

/// Computes the cosine similarity between two normalized or unnormalized vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = (norm_a.sqrt() * norm_b.sqrt()).max(1e-9);
    (dot / denom).clamp(0.0, 1.0)
}

/// A zero-dependency, deterministic subword and character n-gram embedding provider.
///
/// Uses the Hashing Trick over 3-grams and subword tokens into a normalized N-dimensional space.
/// Provides offline morphological and topical similarity without requiring multi-gigabyte models.
#[derive(Debug, Clone)]
pub struct DeterministicSubwordEmbedding {
    dimension: usize,
}

impl DeterministicSubwordEmbedding {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension: dimension.max(64),
        }
    }
}

impl Default for DeterministicSubwordEmbedding {
    fn default() -> Self {
        Self::new(128)
    }
}

impl EmbeddingProvider for DeterministicSubwordEmbedding {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut vector = vec![0.0f32; self.dimension];
        let lower = text.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();

        if words.is_empty() {
            return Ok(vector);
        }

        for word in words {
            // Whole word hash
            hash_feature_into(word, 2.0, &mut vector);

            // Subword 3-grams and 4-grams
            let chars: Vec<char> = word.chars().collect();
            if chars.len() >= 3 {
                for window in chars.windows(3) {
                    let s: String = window.iter().collect();
                    hash_feature_into(&s, 1.0, &mut vector);
                }
            }
            if chars.len() >= 4 {
                for window in chars.windows(4) {
                    let s: String = window.iter().collect();
                    hash_feature_into(&s, 1.2, &mut vector);
                }
            }
        }

        // L2 Normalize
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
        for x in &mut vector {
            *x /= norm;
        }

        Ok(vector)
    }
}

fn hash_feature_into(feature: &str, weight: f32, vector: &mut [f32]) {
    let mut hasher = Sha256::new();
    hasher.update(feature.as_bytes());
    let hash = hasher.finalize();

    let dim = vector.len();
    let bucket = (u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]) as usize) % dim;
    let sign = if hash[4] & 1 == 0 { 1.0 } else { -1.0 };

    vector[bucket] += sign * weight;
}
