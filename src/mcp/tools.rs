//! MCP Tool parameter and result models for DocuGraph.

use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

/// Parameters for `document_ping`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PingParams {
    /// Optional message to echo back
    pub message: Option<String>,
}

/// Parameters for `document_info`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentInfoParams {
    /// Unique identifier or content hash of the document
    pub document_id: String,
}

/// Parameters for `document_outline`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentOutlineParams {
    /// Identifier of the document
    pub document_id: String,
    /// Maximum heading depth to include (e.g. 1 for chapters only, 2 for H1+H2, default: 3)
    pub max_depth: Option<u32>,
}

/// Parameters for `document_search`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentSearchParams {
    /// Search keywords or technical terms
    pub query: String,
    /// Optional filter for a specific document ID (unknown IDs return an error)
    pub document_id: Option<String>,
    /// Maximum number of results to return (default: 5)
    pub limit: Option<usize>,
}

/// Parameters for `document_search_hybrid`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentSearchHybridParams {
    /// Conceptual or natural language query
    pub query: String,
    /// Optional filter for a specific document ID (unknown IDs return an error)
    pub document_id: Option<String>,
    /// Maximum number of results to return (default: 5)
    pub limit: Option<usize>,
    /// Weight for BM25 keyword score (0.0 to 1.0, default: 0.5)
    pub bm25_weight: Option<f32>,
    /// Weight for semantic vector similarity (0.0 to 1.0, default: 0.3)
    pub semantic_weight: Option<f32>,
    /// Weight for structural heading matches (0.0 to 1.0, default: 0.2)
    pub structural_weight: Option<f32>,
}

/// Parameters for `document_get_section`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetSectionParams {
    /// Document identifier
    pub document_id: String,
    /// Identifier or title slug of the target section
    pub section_id: String,
    /// Include parent section header context (default: true)
    pub include_parent: Option<bool>,
    /// Maximum estimated tokens to return (default: 1500)
    pub max_tokens: Option<usize>,
}

/// Parameters for `document_get_context`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetContextParams {
    /// Target topic, concept, or section title
    pub query: String,
    /// Optional document identifier filter (unknown IDs return an error)
    pub document_id: Option<String>,
    /// Maximum estimated tokens in the response (default: 1500)
    pub max_tokens: Option<usize>,
    /// Maximum number of chunks to include (default: 5)
    pub max_chunks: Option<usize>,
}

/// Parameters for `document_get_evidence`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetEvidenceParams {
    /// Claim, assertion, or question to gather verifiable evidence for
    pub query: String,
    /// Optional document identifier filter (unknown IDs return an error)
    pub document_id: Option<String>,
    /// Maximum tokens budget for evidence (default: 1200)
    pub max_tokens: Option<usize>,
    /// Maximum evidence snippets (default: 4)
    pub max_items: Option<usize>,
}

/// Parameters for `document_read_pages`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentReadPagesParams {
    /// Document identifier
    pub document_id: String,
    /// Start page number (1-indexed, inclusive)
    pub page_start: u32,
    /// End page number (1-indexed, inclusive)
    pub page_end: u32,
    /// Maximum characters to return to prevent context explosion (default: 8000)
    pub max_chars: Option<usize>,
}

/// Document summary item returned by `document_list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentSummary {
    pub id: String,
    pub title: String,
    pub total_pages: u32,
    pub total_sections: u32,
    pub content_hash: String,
    pub indexed_at: String,
    pub is_encrypted: bool,
    pub untrusted_text_detected: bool,
    pub scanned_pages_count: u32,
}

/// Detailed structural outline returned by `document_info`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentInfoResult {
    pub id: String,
    pub title: String,
    pub total_pages: u32,
    pub total_sections: u32,
    pub content_hash: String,
    pub sections_preview: Vec<String>,
    pub is_encrypted: bool,
    pub untrusted_text_detected: bool,
    pub scanned_pages_count: u32,
    pub scan_warning: Option<String>,
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

/// Outline node returned by `document_outline`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutlineNodeResult {
    pub id: String,
    pub title: String,
    pub level: u32,
    pub page_start: u32,
    pub page_end: u32,
    pub children: Vec<OutlineNodeResult>,
}

/// Parameters for `document_render_page`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderPageParams {
    /// Document identifier or content hash
    pub document_id: String,
    /// 1-based page number to render
    pub page_number: u32,
    /// Maximum image width in pixels (default: 1024, min: 200, max: 2048)
    pub max_width: Option<u32>,
}

/// Result returned by `document_render_page`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenderPageResult {
    /// Document identifier
    pub document_id: String,
    /// 1-based page number rendered
    pub page_number: u32,
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// MIME type (always "image/png")
    pub mime_type: String,
    /// Base64-encoded RFC-2083 standard PNG image data
    pub base64_image: String,
    /// Complete data URI scheme (e.g. data:image/png;base64,...)
    pub data_uri: String,
    /// Whether the rendered image was served from disk cache (GoF Proxy pattern)
    pub from_cache: bool,
}

/// Parameters for `document_get_links`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetLinksParams {
    /// Document identifier or content hash
    pub document_id: String,
    /// Optional 1-based page number to filter links on a specific page
    pub page: Option<u32>,
    /// Optional filter kind: "all", "external", or "internal" (default: "all")
    pub kind: Option<String>,
}

/// A hyperlink or cross-reference result item.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentLinkResult {
    /// 1-based page number where the link is located
    pub page_number: u32,
    /// Type of link: "external", "internal", or "named"
    pub kind: String,
    /// External destination URL (if external link)
    pub uri: Option<String>,
    /// Target 1-based page number (if internal cross-reference)
    pub target_page: Option<u32>,
    /// Target named anchor or destination identifier (if unresolved to a page)
    pub named_target: Option<String>,
    /// Bounding box rectangle `[x0, y0, x1, y1]` on the source page if present
    pub rect: Option<[f32; 4]>,
}

/// Complete result returned by `document_get_links`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetLinksResult {
    pub document_id: String,
    pub total_links: usize,
    pub links: Vec<DocumentLinkResult>,
}

/// Parameters for `document_get_forms`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetFormsParams {
    /// Document identifier or content hash
    pub document_id: String,
    /// Optional 1-based page number to filter form fields on a specific page
    pub page: Option<u32>,
    /// Optional filter: only return fields that have an assigned non-empty value (default: false)
    pub filled_only: Option<bool>,
}

/// An interactive form field result item.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FormFieldResult {
    /// Short field name
    pub name: String,
    /// Fully qualified hierarchical field name (e.g. "applicant.address.city")
    pub fully_qualified_name: String,
    /// Field type: "text", "checkbox", "radio", "choice", "signature", or "unknown"
    pub field_type: String,
    /// Current assigned value if set
    pub value: Option<String>,
    /// Default value if specified
    pub default_value: Option<String>,
    /// Whether the field is read-only
    pub read_only: bool,
    /// Whether the field is required
    pub required: bool,
    /// 1-based page number where the field widget resides
    pub page_number: Option<u32>,
    /// Bounding box rectangle `[x0, y0, x1, y1]` on the page if present
    pub rect: Option<[f32; 4]>,
}

/// Complete result returned by `document_get_forms`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetFormsResult {
    pub document_id: String,
    pub total_fields: usize,
    pub fields: Vec<FormFieldResult>,
}

/// Parameters for `document_get_attachments`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetAttachmentsParams {
    /// Document identifier or content hash
    pub document_id: String,
}

/// Metadata summary of an embedded file attachment.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AttachmentSummaryResult {
    /// Identifier of the attachment (e.g. "att_factur-x.xml")
    pub id: String,
    /// Filename
    pub filename: String,
    /// Human-readable description if present
    pub description: Option<String>,
    /// MIME type if present (e.g. "text/xml", "application/pdf")
    pub mime_type: Option<String>,
    /// File size in bytes
    pub size_bytes: u64,
    /// MD5 checksum if specified
    pub checksum_md5: Option<String>,
    /// Modification date if specified
    pub mod_date: Option<String>,
    /// Whether the file content is valid UTF-8 text
    pub is_text: bool,
    /// 1-based page number if associated with a page annotation
    pub page_number: Option<u32>,
}

/// Complete result returned by `document_get_attachments`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentGetAttachmentsResult {
    pub document_id: String,
    pub total_attachments: usize,
    pub attachments: Vec<AttachmentSummaryResult>,
}

/// Parameters for `document_read_attachment`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentReadAttachmentParams {
    /// Document identifier or content hash
    pub document_id: String,
    /// Filename or attachment identifier to read
    pub name_or_id: String,
    /// Maximum bytes of content to return (default: 524288 = 512KB)
    pub max_bytes: Option<usize>,
    /// Force output encoding: "text" (UTF-8, default if text) or "base64"
    pub encoding: Option<String>,
}

/// Result returned by `document_read_attachment`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentReadAttachmentResult {
    pub document_id: String,
    pub filename: String,
    pub mime_type: Option<String>,
    pub size_bytes: u64,
    pub encoding: String,
    pub content: String,
    pub truncated: bool,
}
