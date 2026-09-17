//! Modular embedding provider abstractions and vector operations.

use anyhow::Result;

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

        // Character offsets, so an n-gram is a slice of the word rather than a
        // freshly allocated String per window. The features are the same strings
        // as before; only where they come from changed.
        let mut starts: Vec<usize> = Vec::new();

        for word in words {
            // Whole word hash
            hash_feature_into(word, 2.0, &mut vector);

            starts.clear();
            starts.extend(word.char_indices().map(|(i, _)| i));
            let chars = starts.len();

            // Subword 3-grams and 4-grams
            for (k, &start) in starts.iter().enumerate() {
                if k + 3 <= chars {
                    let end = starts.get(k + 3).copied().unwrap_or(word.len());
                    hash_feature_into(&word[start..end], 1.0, &mut vector);
                }
                if k + 4 <= chars {
                    let end = starts.get(k + 4).copied().unwrap_or(word.len());
                    hash_feature_into(&word[start..end], 1.2, &mut vector);
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

/// Place one feature into its bucket with its sign.
///
/// The hashing trick needs the features spread evenly across the buckets. It does
/// not need preimage or collision resistance: there is no adversary choosing
/// n-grams, and only the bucket index and one sign bit ever leave this function.
/// SHA-256 was doing 64 rounds over a 64-byte block to scatter strings like "est",
/// once per word and once per 3-gram and 4-gram of that word.
///
/// FNV-1a plus a splitmix finalizer buys the same uniformity for a few
/// instructions. The finalizer is what makes the choice safe rather than merely
/// cheap: FNV-1a alone mixes its low bits well but its high bits poorly on short
/// inputs, and the bucket and the sign must both be well distributed.
fn hash_feature_into(feature: &str, weight: f32, vector: &mut [f32]) {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET;
    for byte in feature.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    // splitmix64 finalizer: full avalanche, so every output bit depends on every
    // input bit.
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    hash ^= hash >> 33;

    let dim = vector.len();
    let bucket = (hash % dim as u64) as usize;
    let sign = if (hash >> 63) & 1 == 0 { 1.0 } else { -1.0 };

    vector[bucket] += sign * weight;
}
