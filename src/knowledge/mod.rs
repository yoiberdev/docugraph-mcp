//! Domain adapters for specialized knowledge extraction.
//!
//! Generic documents are handled natively without adapters. Domain-specific
//! adapters (e.g., Design Patterns, API specs) provide semantic enrichments.

pub mod adapter;
pub mod design_patterns;

pub use adapter::DomainAdapter;
pub use design_patterns::{DesignPatternsAdapter, ExtractedPattern};
