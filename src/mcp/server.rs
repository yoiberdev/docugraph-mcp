//! DocuGraph Model Context Protocol (MCP) server implementation.

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use tracing::info;

use super::tools::{DocumentInfoParams, DocumentInfoResult, DocumentSummary, PingParams};
use crate::document::{Document, DocumentId};

/// DocuGraph MCP server holding the tool router and shared document knowledge state.
#[derive(Clone)]
pub struct DocuGraphServer {
    tool_router: ToolRouter<Self>,
    documents: Arc<RwLock<HashMap<DocumentId, Document>>>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DocuGraphServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info.name = "docugraph-mcp".to_string();
        info.server_info.version = env!("CARGO_PKG_VERSION").to_string();
        info.instructions = Some(
            "DocuGraph MCP server: query structured knowledge graphs of complex technical PDFs with precise evidence and page provenance."
                .to_string(),
        );
        info
    }
}

impl Default for DocuGraphServer {
    fn default() -> Self {
        Self::new()
    }
}

impl DocuGraphServer {
    /// Create a new server instance with the auto-generated tool router and empty document store.
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register or update a document in the server's in-memory store.
    pub async fn register_document(&self, doc: Document) {
        let mut docs = self.documents.write().await;
        docs.insert(doc.id.clone(), doc);
    }

    /// Run the server over stdio transport until completion or termination signal.
    pub async fn serve_stdio() -> anyhow::Result<()> {
        info!("Initializing DocuGraph MCP server on stdio transport");
        let server = Self::new();
        let service = server.serve(rmcp::transport::stdio()).await?;
        info!("DocuGraph MCP stdio connection established. Ready for JSON-RPC messages.");
        service.waiting().await?;
        info!("DocuGraph MCP stdio session terminated cleanly.");
        Ok(())
    }
}

#[tool_router(router = tool_router)]
impl DocuGraphServer {
    /// Ping the DocuGraph MCP server to verify health, latency, and connectivity.
    #[tool(
        name = "document_ping",
        description = "Check server health and verify stdio connectivity."
    )]
    pub async fn document_ping(&self, params: Parameters<PingParams>) -> String {
        let msg = params
            .0
            .message
            .unwrap_or_else(|| "DocuGraph MCP is alive and ready".to_string());
        format!("pong: {msg}")
    }

    /// List all indexed documents in the local DocuGraph knowledge base.
    #[tool(
        name = "document_list",
        description = "List all indexed PDF documents currently available in the knowledge graph."
    )]
    pub async fn document_list(&self) -> String {
        let docs = self.documents.read().await;
        if docs.is_empty() {
            let sample: Vec<DocumentSummary> = vec![DocumentSummary {
                id: "empty_knowledge_base".to_string(),
                title:
                    "DocuGraph Initialized (No PDFs indexed yet. Run 'docugraph index <file.pdf>')"
                        .to_string(),
                total_pages: 0,
                indexed_at: "2026-09-13T00:00:00Z".to_string(),
            }];
            serde_json::to_string_pretty(&sample).unwrap_or_else(|_| "[]".to_string())
        } else {
            let list: Vec<DocumentSummary> = docs
                .values()
                .map(|d| DocumentSummary {
                    id: d.id.to_string(),
                    title: d.metadata.title.clone(),
                    total_pages: d.metadata.total_pages,
                    indexed_at: d.metadata.indexed_at.clone(),
                })
                .collect();
            serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_string())
        }
    }

    /// Retrieve detailed structural metadata and outline for a specific document.
    #[tool(
        name = "document_info",
        description = "Get structural metadata, page count, and section outline for an indexed document."
    )]
    pub async fn document_info(&self, params: Parameters<DocumentInfoParams>) -> String {
        let doc_id = &params.0.document_id;
        let docs = self.documents.read().await;

        if let Some(doc) = docs.get(&DocumentId(doc_id.clone())) {
            let preview: Vec<String> = doc
                .sections
                .iter()
                .flat_map(|s| s.flatten())
                .take(20)
                .map(|s| {
                    let indent = "  ".repeat((s.level.saturating_sub(1)) as usize);
                    format!(
                        "{indent}* {} (pp. {}-{})",
                        s.title, s.page_start, s.page_end
                    )
                })
                .collect();

            let info = DocumentInfoResult {
                id: doc.id.to_string(),
                title: doc.metadata.title.clone(),
                total_pages: doc.metadata.total_pages,
                total_sections: doc.total_sections() as u32,
                sections_preview: preview,
            };
            serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string())
        } else {
            let info = DocumentInfoResult {
                id: doc_id.clone(),
                title: format!("Document Not Found: {}", doc_id),
                total_pages: 0,
                total_sections: 0,
                sections_preview: vec![
                    "Document ID not found. Use 'document_list' to view available documents."
                        .to_string(),
                ],
            };
            serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string())
        }
    }
}
