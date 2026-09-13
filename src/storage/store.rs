//! DocumentStore provides thread-safe in-memory and disk persistence for documents.

use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::debug;

use super::cache::DiskCache;
use crate::document::model::{Document, DocumentId, DocumentMetadata};

/// Unified document store with fast in-memory access and persistent disk caching.
#[derive(Debug, Clone)]
pub struct DocumentStore {
    memory: Arc<RwLock<HashMap<DocumentId, Document>>>,
    disk: Option<DiskCache>,
}

impl DocumentStore {
    /// Create a new in-memory document store with optional disk caching.
    pub fn new(disk: Option<DiskCache>) -> Self {
        let store = Self {
            memory: Arc::new(RwLock::new(HashMap::new())),
            disk,
        };

        // If disk cache is present, preload metadata or existing cache index
        if let Some(metadata_list) = store.disk.as_ref().and_then(|c| c.list_metadata().ok()) {
            debug!("Found {} cached documents on disk", metadata_list.len());
        }

        store
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
        mem.insert(doc.id.clone(), doc);
        Ok(())
    }

    /// Get a document by its ID, checking memory first, then disk cache.
    pub fn get(&self, id: &str) -> Option<Document> {
        // 1. Check in-memory
        if let Some(doc) = self.memory.read().ok().and_then(|mem| mem.get(id).cloned()) {
            return Some(doc);
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
                        mem.insert(doc.id.clone(), doc.clone());
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
        if let Ok(mem) = self.memory.read() {
            for doc in mem.values() {
                if doc.metadata.content_hash == hash {
                    return Some(doc.clone());
                }
            }
        }

        // 2. Check disk
        if let Some(doc) = self
            .disk
            .as_ref()
            .and_then(|c| c.load_by_hash(hash).ok().flatten())
        {
            if let Ok(mut mem) = self.memory.write() {
                mem.insert(doc.id.clone(), doc.clone());
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

        // From memory (takes precedence if more up-to-date)
        if let Ok(mem) = self.memory.read() {
            for doc in mem.values() {
                map.insert(doc.metadata.id.clone(), doc.metadata.clone());
            }
        }

        map.into_values().collect()
    }
}
