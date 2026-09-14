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

/// Target destination type of a link found within a document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "value")]
pub enum LinkTarget {
    /// External web link (HTTP / HTTPS / mailto, etc.)
    Uri(String),
    /// Internal jump to a 1-based page number
    InternalPage(u32),
    /// Named destination that could not be resolved to a specific page number
    Named(String),
}

/// A hyperlink or internal cross-reference extracted from a document page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocumentLink {
    /// 1-based page number where the link is located
    pub page_number: u32,
    /// Target destination of the link
    pub target: LinkTarget,
    /// Bounding box rectangle [x0, y0, x1, y1] on the source page if present
    pub rect: Option<[f32; 4]>,
    /// External URI string if this is an external link
    pub uri: Option<String>,
    /// Target page number if this is an internal jump
    pub target_page: Option<u32>,
}

impl DocumentLink {
    pub fn uri(page_number: u32, uri: impl Into<String>, rect: Option<[f32; 4]>) -> Self {
        let u = uri.into();
        Self {
            page_number,
            target: LinkTarget::Uri(u.clone()),
            rect,
            uri: Some(u),
            target_page: None,
        }
    }

    pub fn internal(page_number: u32, target_page: u32, rect: Option<[f32; 4]>) -> Self {
        Self {
            page_number,
            target: LinkTarget::InternalPage(target_page),
            rect,
            uri: None,
            target_page: Some(target_page),
        }
    }

    pub fn named(page_number: u32, name: impl Into<String>, rect: Option<[f32; 4]>) -> Self {
        let n = name.into();
        Self {
            page_number,
            target: LinkTarget::Named(n),
            rect,
            uri: None,
            target_page: None,
        }
    }

    pub fn is_external(&self) -> bool {
        matches!(self.target, LinkTarget::Uri(_))
    }

    pub fn is_internal(&self) -> bool {
        matches!(self.target, LinkTarget::InternalPage(_))
    }
}

/// Type of an interactive form field extracted from an /AcroForm dictionary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "details")]
pub enum FormFieldType {
    /// Text input field (/Tx)
    Text,
    /// Checkbox toggle field (/Btn)
    Checkbox,
    /// Radio button group choice (/Btn with radio flag)
    Radio,
    /// Push button (/Btn with pushbutton flag)
    Button,
    /// Choice dropdown or list box (/Ch)
    Choice,
    /// Digital signature field (/Sig)
    Signature,
    /// Unknown or custom field type
    Unknown(String),
}

impl FormFieldType {
    pub fn as_str(&self) -> &str {
        match self {
            FormFieldType::Text => "text",
            FormFieldType::Checkbox => "checkbox",
            FormFieldType::Radio => "radio",
            FormFieldType::Button => "button",
            FormFieldType::Choice => "choice",
            FormFieldType::Signature => "signature",
            FormFieldType::Unknown(s) => s.as_str(),
        }
    }
}

/// An interactive form field extracted from a PDF's /AcroForm hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormField {
    /// Simple field name (/T)
    pub name: String,
    /// Fully qualified hierarchical field name (e.g. "applicant.address.city")
    pub fully_qualified_name: String,
    /// Classified field type
    pub field_type: FormFieldType,
    /// Current assigned value (/V)
    pub value: Option<String>,
    /// Default value (/DV)
    pub default_value: Option<String>,
    /// Whether the field is read-only (bit 1 of /Ff)
    pub read_only: bool,
    /// Whether the field is required (bit 2 of /Ff)
    pub required: bool,
    /// 1-based page number where the visual field widget is located
    pub page_number: Option<u32>,
    /// Bounding box rectangle [x0, y0, x1, y1] on the page if present
    pub rect: Option<[f32; 4]>,
}

/// An embedded file or attachment extracted from a PDF (/EmbeddedFiles, /AF, or /FileAttachment).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddedAttachment {
    /// Unique identifier for the attachment (e.g. "att_factur-x.xml")
    pub id: String,
    /// Filename (extracted from /UF or /F)
    pub filename: String,
    /// Optional human-readable description (/Desc)
    pub description: Option<String>,
    /// MIME subtype if specified (/Subtype, e.g. "text/xml", "application/pdf")
    pub mime_type: Option<String>,
    /// Uncompressed file size in bytes
    pub size_bytes: u64,
    /// MD5 checksum hex string if specified in /Params /CheckSum
    pub checksum_md5: Option<String>,
    /// Modification date string if specified (/Params /ModDate)
    pub mod_date: Option<String>,
    /// Whether the content is valid UTF-8 text (e.g. XML, CSV, JSON, TXT)
    pub is_text: bool,
    /// 1-based page number if this attachment is associated with a page annotation
    pub page_number: Option<u32>,
    /// Extracted raw content bytes (decompressed)
    #[serde(default)]
    pub data: Vec<u8>,
}

impl EmbeddedAttachment {
    /// Return the content decoded as UTF-8 string if valid, or None if binary.
    pub fn text_content(&self) -> Option<&str> {
        if self.is_text {
            std::str::from_utf8(&self.data).ok()
        } else {
            None
        }
    }
}

/// Metadata extracted from a document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(default)]
    pub total_links: u32,
    #[serde(default)]
    pub has_forms: bool,
    #[serde(default)]
    pub total_form_fields: u32,
    #[serde(default)]
    pub is_tagged: bool,
    #[serde(default)]
    pub has_attachments: bool,
    #[serde(default)]
    pub total_attachments: u32,
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
    /// Hyperlinks extracted from this page (external URIs and internal cross-references)
    #[serde(default)]
    pub links: Vec<DocumentLink>,
    /// Printed page label from the PDF `/PageLabels` tree (e.g. "iii", "17", "A-2"), if the
    /// document defines a non-empty one. `page_number` stays the physical PDF position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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
            links: Vec::new(),
            label: None,
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
    #[serde(default)]
    pub forms: Vec<FormField>,
    #[serde(default)]
    pub attachments: Vec<EmbeddedAttachment>,
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
            forms: Vec::new(),
            attachments: Vec::new(),
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

    /// Printed label of a page (from `/PageLabels`), if the document defines a non-empty one.
    pub fn page_label(&self, page_number: u32) -> Option<&str> {
        self.get_page(page_number)
            .and_then(|page| page.label.as_deref())
    }

    /// Whether any page carries a printed label from `/PageLabels`.
    pub fn has_page_labels(&self) -> bool {
        self.pages.iter().any(|page| page.label.is_some())
    }

    /// Return all hyperlinks extracted across all pages in this document.
    pub fn all_links(&self) -> Vec<&DocumentLink> {
        self.pages.iter().flat_map(|p| &p.links).collect()
    }

    /// Return all hyperlinks extracted for a specific page number.
    pub fn links_for_page(&self, page_number: u32) -> Vec<&DocumentLink> {
        self.get_page(page_number)
            .map(|p| p.links.iter().collect())
            .unwrap_or_default()
    }

    /// Return a slice of all interactive form fields found in this document.
    pub fn forms(&self) -> &[FormField] {
        &self.forms
    }

    /// Return all interactive form fields mapped to a specific page number.
    pub fn forms_for_page(&self, page_number: u32) -> Vec<&FormField> {
        self.forms
            .iter()
            .filter(|f| f.page_number == Some(page_number))
            .collect()
    }

    /// Return a slice of all embedded file attachments in this document.
    pub fn attachments(&self) -> &[EmbeddedAttachment] {
        &self.attachments
    }

    /// Find an embedded attachment by its filename or ID (case-insensitive).
    pub fn get_attachment(&self, name_or_id: &str) -> Option<&EmbeddedAttachment> {
        self.attachments.iter().find(|a| {
            a.filename.eq_ignore_ascii_case(name_or_id) || a.id.eq_ignore_ascii_case(name_or_id)
        })
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
