//! Core data models representing documents, pages, and hierarchical section graphs.

use serde::{Deserialize, Serialize};

/// Strongly-typed identifier for an indexed document.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub String);

impl std::fmt::Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Metadata extracted from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub title: String,
    pub author: Option<String>,
    pub total_pages: u32,
    pub file_size_bytes: u64,
    pub sha256_hash: String,
    pub indexed_at: String,
}

/// A single extracted page from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    /// 1-based page number
    pub page_number: u32,
    /// Extracted textual content of the page
    pub text: String,
    /// Number of characters extracted
    pub char_count: usize,
}

/// A node in the hierarchical document outline / graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionNode {
    /// Normalized unique identifier for the section (e.g. "sec-1-2" or "strategy-consequences")
    pub id: String,
    /// Human-readable title of the heading/section
    pub title: String,
    /// Heading depth level (1 for Chapter/H1, 2 for H2, 3 for H3)
    pub level: u32,
    /// 1-based start page
    pub page_start: u32,
    /// 1-based end page (inclusive)
    pub page_end: u32,
    /// Parent section ID if nested
    pub parent_id: Option<String>,
    /// Child subsections
    pub children: Vec<SectionNode>,
    /// Brief preview of the text content under this section
    pub content_preview: String,
}

impl SectionNode {
    /// Count total sections including this node and all descendant nodes recursively.
    pub fn total_count(&self) -> usize {
        1 + self.children.iter().map(|c| c.total_count()).sum::<usize>()
    }

    /// Flatten this section and all children into a pre-order list.
    pub fn flatten(&self) -> Vec<&SectionNode> {
        let mut list = vec![self];
        for child in &self.children {
            list.extend(child.flatten());
        }
        list
    }
}

/// Complete in-memory representation of an ingested document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub metadata: DocumentMetadata,
    pub pages: Vec<Page>,
    pub sections: Vec<SectionNode>,
}

impl Document {
    /// Get the total count of sections across the entire document outline.
    pub fn total_sections(&self) -> usize {
        self.sections.iter().map(|s| s.total_count()).sum()
    }

    /// Retrieve the text of a specific page (1-based index).
    pub fn get_page(&self, page_number: u32) -> Option<&Page> {
        self.pages.iter().find(|p| p.page_number == page_number)
    }

    /// Find a section by its unique section ID.
    pub fn find_section(&self, section_id: &str) -> Option<&SectionNode> {
        for root in &self.sections {
            for s in root.flatten() {
                if s.id == section_id {
                    return Some(s);
                }
            }
        }
        None
    }
}
