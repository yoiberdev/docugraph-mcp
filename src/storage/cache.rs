//! Disk caching for parsed documents using content-hash indexing.

use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use crate::document::model::{Document, DocumentMetadata};

/// Persistent document cache on disk.
#[derive(Debug, Clone)]
pub struct DiskCache {
    cache_dir: PathBuf,
}

impl DiskCache {
    /// Create a new disk cache in the specified directory.
    pub fn new(cache_dir: impl AsRef<Path>) -> Result<Self> {
        let dir = cache_dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)
            .with_context(|| format!("Failed to create cache directory at {:?}", dir))?;
        Ok(Self { cache_dir: dir })
    }

    /// Default cache directory in current working directory or system temp.
    pub fn default_dir() -> PathBuf {
        std::env::var("DOCUGRAPH_CACHE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(".docugraph_cache"))
    }

    /// Path to a cached document JSON file by SHA-256 hash.
    fn cache_path_for_hash(&self, content_hash: &str) -> PathBuf {
        self.cache_dir.join(format!("{}.json", content_hash))
    }

    /// Save a document to disk cache using its SHA-256 content hash as key.
    pub fn save(&self, doc: &Document) -> Result<()> {
        let path = self.cache_path_for_hash(&doc.metadata.content_hash);
        debug!("Caching document '{}' to {:?}", doc.metadata.id, path);
        let file = File::create(&path)
            .with_context(|| format!("Failed to create cache file at {:?}", path))?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, doc)
            .with_context(|| format!("Failed to serialize document to {:?}", path))?;
        info!(
            "Successfully cached document '{}' ({} pages) to disk",
            doc.metadata.id, doc.metadata.total_pages
        );
        Ok(())
    }

    /// Load a document from disk cache given its SHA-256 content hash.
    pub fn load_by_hash(&self, content_hash: &str) -> Result<Option<Document>> {
        let path = self.cache_path_for_hash(content_hash);
        if !path.exists() {
            return Ok(None);
        }

        let file = File::open(&path)
            .with_context(|| format!("Failed to open cache file at {:?}", path))?;
        let reader = BufReader::new(file);
        let doc: Document = serde_json::from_reader(reader)
            .with_context(|| format!("Failed to deserialize document from {:?}", path))?;
        debug!(
            "Loaded cached document '{}' from {:?}",
            doc.metadata.id, path
        );
        Ok(Some(doc))
    }

    /// List all cached documents metadata without loading full page text.
    pub fn list_metadata(&self) -> Result<Vec<DocumentMetadata>> {
        let mut list = Vec::new();
        if !self.cache_dir.exists() {
            return Ok(list);
        }

        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            let path = entry.path();
            let is_json = path.extension().and_then(|e| e.to_str()) == Some("json");
            if !is_json {
                continue;
            }
            if let Ok(file) = File::open(&path) {
                let reader = BufReader::new(file);
                if let Ok(doc) = serde_json::from_reader::<_, Document>(reader) {
                    list.push(doc.metadata);
                }
            }
        }
        Ok(list)
    }

    /// Remove a cached document by its content hash.
    pub fn remove_by_hash(&self, content_hash: &str) -> Result<bool> {
        let path = self.cache_path_for_hash(content_hash);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
