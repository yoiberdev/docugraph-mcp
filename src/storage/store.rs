//! DocumentStore provides thread-safe in-memory and disk persistence for documents.

use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::debug;

use super::cache::DiskCache;
use crate::document::model::{Document, DocumentId, DocumentMetadata};

/// How much page text the memory tier holds before it evicts.
///
/// Measured on a corpus of five public documents, 6576 pages and 4.96 million
/// tokens: one corpus-wide query left the server resident at 513 MB, and nothing
/// ever released it. The tier had no eviction path at all - no `remove`, no
/// `clear`, no `retain` - so a server left running grew to hold whatever had been
/// asked about and stayed there for the life of the process.
///
/// A budget only makes sense because eviction is recoverable: a document evicted
/// here is still in the disk cache and reloads on the next request, so the cost is
/// a reload rather than a loss. Where there is no disk cache there is nowhere to
/// reload from, so that store stays unbounded on purpose - see [`MemoryTier::budget`].
const DEFAULT_MEMORY_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// The in-memory tier: the documents held, and what it takes to decide which to drop.
#[derive(Debug, Default)]
struct MemoryTier {
    docs: HashMap<DocumentId, Document>,
    /// The value of `clock` when each document was last read or written.
    used_at: HashMap<DocumentId, u64>,
    /// Monotonic tick, incremented on every access, so "least recently used" is
    /// decidable without a wall clock.
    clock: u64,
    /// Page text currently held, in bytes.
    bytes: usize,
    /// The ceiling `bytes` is kept under. Zero means unbounded, which is what a
    /// store without a disk cache uses: evicting there would lose the document.
    budget: usize,
}

/// What a document costs to keep resident.
///
/// Page text dominates a parsed document by an order of magnitude over its
/// section tree and metadata, so it is what the budget counts. This is an
/// approximation of resident size, not a measurement of it, and it is used to
/// compare documents against each other rather than to predict RSS.
fn resident_bytes(doc: &Document) -> usize {
    doc.pages.iter().map(|p| p.text.len()).sum()
}

impl MemoryTier {
    fn with_budget(budget: usize) -> Self {
        Self {
            budget,
            ..Default::default()
        }
    }

    fn touch(&mut self, id: &DocumentId) {
        self.clock += 1;
        self.used_at.insert(id.clone(), self.clock);
    }

    fn get(&mut self, id: &str) -> Option<Document> {
        let found = self.docs.get(id).cloned()?;
        self.touch(&found.id);
        Some(found)
    }

    fn get_by_hash(&mut self, hash: &str) -> Option<Document> {
        let found = self
            .docs
            .values()
            .find(|d| d.metadata.content_hash == hash)
            .cloned()?;
        self.touch(&found.id);
        Some(found)
    }

    fn insert(&mut self, doc: Document) {
        let id = doc.id.clone();
        if let Some(previous) = self.docs.insert(id.clone(), doc) {
            self.bytes = self.bytes.saturating_sub(resident_bytes(&previous));
        }
        self.bytes += self
            .docs
            .get(&id)
            .map(resident_bytes)
            .unwrap_or_default();
        self.touch(&id);
        self.evict_to_budget();
    }

    /// Drop least-recently-used documents until the tier is within budget.
    ///
    /// The document just inserted is the most recently used, so it is never the
    /// one dropped - a document larger than the whole budget stays resident alone
    /// rather than being evicted immediately and reloaded on the next call.
    fn evict_to_budget(&mut self) {
        if self.budget == 0 {
            return;
        }
        while self.bytes > self.budget && self.docs.len() > 1 {
            let Some(victim) = self
                .used_at
                .iter()
                .filter(|(id, _)| self.docs.contains_key(*id))
                .min_by_key(|(_, used)| **used)
                .map(|(id, _)| id.clone())
            else {
                return;
            };
            if let Some(dropped) = self.docs.remove(&victim) {
                self.bytes = self.bytes.saturating_sub(resident_bytes(&dropped));
                debug!(
                    document = %dropped.metadata.id,
                    bytes = resident_bytes(&dropped),
                    resident = self.bytes,
                    "Evicted a document from memory; it reloads from the disk cache on next use"
                );
            }
            self.used_at.remove(&victim);
        }
    }
}

/// Unified document store with fast in-memory access and persistent disk caching.
#[derive(Debug, Clone)]
pub struct DocumentStore {
    memory: Arc<RwLock<MemoryTier>>,
    disk: Option<DiskCache>,
}

impl DocumentStore {
    /// Create a new in-memory document store with optional disk caching.
    pub fn new(disk: Option<DiskCache>) -> Self {
        // Without a disk cache an evicted document is gone, so that store is
        // unbounded: the alternative is losing data the caller handed us.
        let budget = if disk.is_some() {
            DEFAULT_MEMORY_BUDGET_BYTES
        } else {
            0
        };

        let store = Self {
            memory: Arc::new(RwLock::new(MemoryTier::with_budget(budget))),
            disk,
        };

        // If disk cache is present, preload metadata or existing cache index
        if let Some(metadata_list) = store.disk.as_ref().and_then(|c| c.list_metadata().ok()) {
            debug!("Found {} cached documents on disk", metadata_list.len());
        }

        store
    }

    /// The absolute directory this store persists to, if it persists at all.
    ///
    /// Worth surfacing rather than keeping internal: when the corpus comes back
    /// empty, the directory that was looked in is the answer to why.
    pub fn cache_dir(&self) -> Option<std::path::PathBuf> {
        self.disk.as_ref().map(|cache| cache.dir())
    }

    /// Page text currently held in memory, in bytes.
    pub fn resident_bytes(&self) -> usize {
        self.memory.read().map(|mem| mem.bytes).unwrap_or_default()
    }

    /// Insert or update a document in memory and optionally persist to disk cache.
    pub fn insert(&self, doc: Document) -> Result<()> {
        if let Some(ref cache) = self.disk {
            let _ = cache.save(&doc).map_err(|e| {
                tracing::warn!(
                    "Failed to persist document '{}' to disk: {}",
                    doc.metadata.id,
                    e
                );
            });
        }

        let mut mem = self
            .memory
            .write()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        mem.insert(doc);
        Ok(())
    }

    /// Get a document by its ID or content hash, checking memory first, then disk cache.
    pub fn get(&self, id: &str) -> Option<Document> {
        // 1. In memory, by id, then by content hash. The disk branch below accepts
        //    either identifier, so without the second a hash would only resolve
        //    when a disk cache is configured.
        if let Ok(mut mem) = self.memory.write() {
            if let Some(doc) = mem.get(id).or_else(|| mem.get_by_hash(id)) {
                return Some(doc);
            }
        }

        // 2. Check disk cache
        if let Some(ref cache) = self.disk {
            let metadata_list = cache.list_metadata().unwrap_or_default();
            for meta in metadata_list {
                if meta.id != id && meta.content_hash != id {
                    continue;
                }
                if let Ok(Some(doc)) = cache.load_by_hash(&meta.content_hash) {
                    // Warm memory cache
                    if let Ok(mut mem) = self.memory.write() {
                        mem.insert(doc.clone());
                    }
                    return Some(doc);
                }
            }
        }

        None
    }

    /// Get a document by its content SHA-256 hash.
    pub fn get_by_hash(&self, hash: &str) -> Option<Document> {
        // 1. Check memory
        if let Ok(mut mem) = self.memory.write()
            && let Some(doc) = mem.get_by_hash(hash)
        {
            return Some(doc);
        }

        // 2. Check disk
        if let Some(doc) = self
            .disk
            .as_ref()
            .and_then(|c| c.load_by_hash(hash).ok().flatten())
        {
            if let Ok(mut mem) = self.memory.write() {
                mem.insert(doc.clone());
            }
            return Some(doc);
        }

        None
    }

    /// List metadata for all loaded and cached documents.
    pub fn list_documents(&self) -> Vec<DocumentMetadata> {
        let mut map: HashMap<String, DocumentMetadata> = HashMap::new();

        // From disk
        if let Some(metas) = self.disk.as_ref().and_then(|c| c.list_metadata().ok()) {
            for m in metas {
                map.insert(m.id.clone(), m);
            }
        }

        // From memory (takes precedence if more up-to-date). Listing is not a use
        // of a document, so it does not touch the eviction order: a `document_list`
        // on every turn would otherwise keep the whole corpus looking hot.
        if let Ok(mem) = self.memory.read() {
            for doc in mem.docs.values() {
                map.insert(doc.metadata.id.clone(), doc.metadata.clone());
            }
        }

        let mut list: Vec<DocumentMetadata> = map.into_values().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }
}
