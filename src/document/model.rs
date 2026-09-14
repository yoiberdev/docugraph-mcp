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

impl std::borrow::Borrow<str> for DocumentId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for DocumentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DocumentId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for DocumentId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Categorization of a page's content layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageKind {
    /// Digital text page containing selectable, searchable text
    DigitalText,
    /// Scanned image page containing bitmap images with little or no digital text layer
    ScannedImage,
    /// Blank / empty page containing neither text nor images
    Empty,
}

/// Metadata extracted from a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub total_pages: u32,
    pub total_sections: u32,
    pub file_size_bytes: u64,
    pub content_hash: String,
    pub indexed_at: String,
    pub is_encrypted: bool,
    pub untrusted_text_detected: bool,
    pub scanned_pages_count: u32,
    #[serde(default)]
    pub source_path: Option<String>,
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
    /// Whether suspicious invisible or microscopic text was detected on this page
    pub untrusted_text_detected: bool,
    /// Classification of the page (digital text, scanned image, or empty)
    pub kind: PageKind,
    /// Number of bitmap images discovered in the page resources
    pub image_count: usize,
}

impl Page {
    /// Helper to construct a clean digital page without untrusted text flags.
    pub fn new(page_number: u32, text: impl Into<String>) -> Self {
        let text = text.into();
        let char_count = text.chars().count();
        Self {
            page_number,
            char_count,
            text,
            untrusted_text_detected: false,
            kind: PageKind::DigitalText,
            image_count: 0,
        }
    }
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
    /// Create a new section node with default empty children and preview.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        level: u32,
        page_start: u32,
        page_end: u32,
        parent_id: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            level,
            page_start,
            page_end,
            parent_id,
            children: Vec::new(),
            content_preview: String::new(),
        }
    }

    /// Add a child section node.
    pub fn add_child(&mut self, child: SectionNode) {
        self.children.push(child);
    }

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
    /// Create a new document with given metadata.
    pub fn new(metadata: DocumentMetadata) -> Self {
        let id = DocumentId(metadata.id.clone());
        Self {
            id,
            metadata,
            pages: Vec::new(),
            sections: Vec::new(),
        }
    }

    /// Add a page to the document.
    pub fn add_page(&mut self, page: Page) {
        self.pages.push(page);
    }

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
