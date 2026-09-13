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
    /// Unique identifier or filename of the document
    pub document_id: String,
}

/// Document summary item returned by `document_list`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentSummary {
    /// Identifier of the document
    pub id: String,
    /// Document title or source file name
    pub title: String,
    /// Total number of pages parsed
    pub total_pages: u32,
    /// ISO 8601 timestamp when the document was indexed
    pub indexed_at: String,
}

/// Detailed structural outline returned by `document_info`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentInfoResult {
    /// Identifier of the document
    pub id: String,
    /// Document title
    pub title: String,
    /// Total number of pages
    pub total_pages: u32,
    /// Total number of sections identified
    pub total_sections: u32,
    /// List of top-level sections / headings preview
    pub sections_preview: Vec<String>,
}
