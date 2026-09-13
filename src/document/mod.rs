//! Document parsing, layout models, and provenance tracking.

use serde::{Deserialize, Serialize};

/// Unique identifier for an indexed document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub String);

/// Provenance metadata tracking exact origin within a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub document_id: DocumentId,
    pub page_number: u32,
    pub section_id: Option<String>,
    pub source_offset: Option<usize>,
}
