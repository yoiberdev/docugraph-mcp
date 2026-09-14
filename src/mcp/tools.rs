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
    /// Optional filter for a specific document ID
    pub document_id: Option<String>,
    /// Maximum number of results to return (default: 5)
    pub limit: Option<usize>,
}

/// Parameters for `document_search_hybrid`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentSearchHybridParams {
    /// Conceptual or natural language query
    pub query: String,
    /// Optional filter for a specific document ID
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
    /// Optional document identifier filter
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
    /// Optional document identifier filter
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
