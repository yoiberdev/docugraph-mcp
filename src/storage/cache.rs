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

    /// Where documents are cached, resolved in three steps.
    ///
    /// This directory is the only thing connecting `docugraph index` to
    /// `docugraph serve`: they are separate processes and the cache is the handoff.
    /// Resolving it relative to the current directory made that handoff depend on
    /// where each process happened to be started, and an MCP client launched
    /// without a fixed `cwd` therefore reported an empty corpus however many PDFs
    /// had been indexed - connected, 15 tools, and nothing in them.
    ///
    /// So: an explicit `DOCUGRAPH_CACHE_DIR` wins; otherwise a `.docugraph_cache`
    /// that already exists beside the current directory is kept, so anyone already
    /// relying on a project-local cache keeps it; otherwise the per-user data
    /// directory, which is the same for every process regardless of where it starts.
    pub fn default_dir() -> PathBuf {
        if let Ok(explicit) = std::env::var("DOCUGRAPH_CACHE_DIR") {
            let dir = PathBuf::from(explicit);
            if !dir.as_os_str().is_empty() {
                return dir;
            }
        }

        let local = PathBuf::from(".docugraph_cache");
        if local.is_dir() {
            return local;
        }

        Self::user_data_dir()
    }

    /// The per-user data directory, found without pulling in a crate for it.
    ///
    /// Falls back to the working directory only if the platform tells us nothing,
    /// which keeps the old behaviour as the last resort rather than the default.
    fn user_data_dir() -> PathBuf {
        let base = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
                })
        };

        match base {
            Some(dir) => dir.join("docugraph").join("cache"),
            None => PathBuf::from(".docugraph_cache"),
        }
    }

    /// The absolute path this cache writes to, for telling the user where its
    /// documents actually live.
    ///
    /// The Windows verbatim prefix is stripped: this path is printed for a person
    /// to read and paste into a config file, and `\\?\C:\...` is neither.
    pub fn dir(&self) -> PathBuf {
        let absolute = fs::canonicalize(&self.cache_dir).unwrap_or_else(|_| self.cache_dir.clone());
        match absolute.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
            Some(plain) => PathBuf::from(plain),
            None => absolute,
        }
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
