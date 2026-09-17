//! Cache proxy over `HybridRetriever::build`.
//!
//! An index is a pure function of the documents it was built from, so the content
//! signature of those documents is its cache key. Same shape as
//! `CachedPageRendererProxy` in `crate::multimodal`, with the cache in memory
//! instead of on disk.
//!
//! Measured on a 437-page manual (488 search units): building costs 537 ms and
//! searching costs 0.46 ms, so the index was being built at roughly 1,100 times
//! the cost of using it and then thrown away, once per tool call. Since lexical
//! admission landed, a query the corpus cannot answer costs the same to refuse as
//! a real one costs to answer, which made "no evidence" a half-second operation.

use std::sync::{Arc, RwLock};

use sha2::{Digest, Sha256};

use super::hybrid::HybridRetriever;
use crate::document::model::Document;

/// Identity of a corpus slice: the ids and content hashes of the documents in
/// scope, in a stable order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusSignature(String);

impl CorpusSignature {
    pub fn of(docs: &[Document]) -> Self {
        let mut identities: Vec<(&str, &str)> = docs
            .iter()
            .map(|d| (d.metadata.id.as_str(), d.metadata.content_hash.as_str()))
            .collect();
        // Sorting makes the scope a set rather than a list: the same corpus keys
        // the same however the caller happened to order it.
        identities.sort_unstable();

        let mut hasher = Sha256::new();
        for (id, content_hash) in identities {
            hasher.update(id.as_bytes());
            hasher.update([0]);
            hasher.update(content_hash.as_bytes());
            hasher.update([0]);
        }
        Self(hex::encode(hasher.finalize()))
    }
}

/// Keeps the last few built indices alive, keyed by corpus signature.
///
/// In memory only, never on disk: `content_hash` identifies the source PDF, not
/// the parser that turned it into search units, so a persisted index would
/// survive a change to tokenization or section extraction that should have
/// invalidated it. Living in the process makes that impossible.
///
/// There is no `invalidate`: the signature *is* the invalidation. A reindexed
/// document gets a new content hash, a removed one drops out of the scope, and
/// either way the key changes and misses. Nothing can be forgotten.
/// One resident index, tagged with the corpus it was built from.
type Slot = (CorpusSignature, Arc<HybridRetriever>);

#[derive(Clone, Default)]
pub struct RetrieverCache {
    slots: Arc<RwLock<Vec<Slot>>>,
}

impl RetrieverCache {
    /// Resident scopes. An agent's working set is one or two - the whole corpus,
    /// then one document - so four leaves room without turning this into a leak.
    /// Each slot costs one index: 8.3 MB for a 437-page book, measured.
    const CAPACITY: usize = 4;

    /// Return the index for this corpus, building it only if no slot holds it.
    pub fn get_or_build(&self, docs: &[Document]) -> Arc<HybridRetriever> {
        let signature = CorpusSignature::of(docs);
        if let Some(hit) = self.lookup(&signature) {
            return hit;
        }

        // Built outside every lock. Two callers racing on the same signature build
        // the same index twice and the second insert is a no-op: that costs CPU in
        // a rare race, where holding the lock across the build would cost half a
        // second of latency to every concurrent reader.
        let built = Arc::new(HybridRetriever::build(docs, None));

        if let Ok(mut slots) = self.slots.write()
            && !slots.iter().any(|(s, _)| *s == signature)
        {
            if slots.len() >= Self::CAPACITY {
                slots.remove(0);
            }
            slots.push((signature, Arc::clone(&built)));
        }
        built
    }

    /// FIFO, not LRU: reordering on a hit would need a write lock on the read
    /// path, and with four slots against a working set of one or two there is
    /// nothing worth reordering.
    fn lookup(&self, signature: &CorpusSignature) -> Option<Arc<HybridRetriever>> {
        let slots = self.slots.read().ok()?;
        slots
            .iter()
            .find(|(s, _)| s == signature)
            .map(|(_, retriever)| Arc::clone(retriever))
    }
}
