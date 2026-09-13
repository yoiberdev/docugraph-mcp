//! Storage and persistence abstractions for documents, cache, and indices.

pub mod cache;
pub mod store;

pub use cache::DiskCache;
pub use store::DocumentStore;
