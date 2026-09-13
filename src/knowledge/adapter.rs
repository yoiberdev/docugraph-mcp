//! Generic domain adapter traits for document semantic enrichment.

use crate::document::model::Document;

/// Trait implemented by domain-specific knowledge extractors.
pub trait DomainAdapter: Send + Sync {
    /// Identifier for this domain adapter (e.g., "design_patterns", "rfc_spec").
    fn domain_id(&self) -> &'static str;

    /// Human-readable name of the domain.
    fn domain_name(&self) -> &'static str;

    /// Check if this document likely belongs to the adapter's domain based on structure or title.
    fn is_applicable(&self, doc: &Document) -> bool;
}
