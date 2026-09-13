//! Domain adapters for specialized knowledge extraction.
//!
//! Generic documents are handled natively without adapters. Domain-specific
//! adapters (e.g., Design Patterns, API specs) provide semantic enrichments.

pub trait DomainAdapter: Send + Sync {
    fn domain_name(&self) -> &'static str;
}
