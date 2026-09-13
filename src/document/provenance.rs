//! Provenance tracking and source citations for agentic reasoning.

use serde::{Deserialize, Serialize};

use super::model::DocumentId;

/// Explicit provenance metadata tracking the exact origin of retrieved knowledge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    /// Identifier of the origin document
    pub document_id: DocumentId,
    /// 1-based page number where the content appears
    pub page_number: u32,
    /// Unique identifier of the enclosing section (if identified)
    pub section_id: Option<String>,
    /// Character or byte offset within the page
    pub source_offset: Option<usize>,
}

impl Provenance {
    /// Create a new provenance citation anchor.
    pub fn new(document_id: DocumentId, page_number: u32) -> Self {
        Self {
            document_id,
            page_number,
            section_id: None,
            source_offset: None,
        }
    }

    /// Add section context to the provenance.
    pub fn with_section(mut self, section_id: impl Into<String>) -> Self {
        self.section_id = Some(section_id.into());
        self
    }

    /// Add offset context to the provenance.
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.source_offset = Some(offset);
        self
    }

    /// Format as a concise, unambiguous Markdown citation for AI agents and users.
    pub fn format_citation(&self) -> String {
        match (&self.section_id, self.source_offset) {
            (Some(sec), Some(off)) => {
                format!(
                    "[Doc: {}, Page: {}, Section: {}, Offset: {}]",
                    self.document_id, self.page_number, sec, off
                )
            }
            (Some(sec), None) => {
                format!(
                    "[Doc: {}, Page: {}, Section: {}]",
                    self.document_id, self.page_number, sec
                )
            }
            (None, Some(off)) => {
                format!(
                    "[Doc: {}, Page: {}, Offset: {}]",
                    self.document_id, self.page_number, off
                )
            }
            (None, None) => {
                format!("[Doc: {}, Page: {}]", self.document_id, self.page_number)
            }
        }
    }
}
