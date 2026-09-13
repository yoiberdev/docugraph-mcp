//! Storage and persistence abstractions for documents, vectors, and graph relations.

use anyhow::Result;

pub trait StorageEngine: Send + Sync {
    fn initialize(&self) -> Result<()>;
}
