//! DocuGraph Model Context Protocol (MCP) server implementation.

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use tracing::info;

use super::tools::*;
use crate::document::model::{Document, SectionNode};
use crate::knowledge::design_patterns::DesignPatternsAdapter;
use crate::retrieval::{ContextBudget, ContextBuilder, HybridRetriever, HybridWeights};
use crate::storage::{DiskCache, DocumentStore};

/// DocuGraph MCP server holding the tool router and shared document store.
#[derive(Clone)]
pub struct DocuGraphServer {
    tool_router: ToolRouter<Self>,
    store: DocumentStore,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DocuGraphServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info.name = "docugraph-mcp".to_string();
        info.server_info.version = env!("CARGO_PKG_VERSION").to_string();
        info.instructions = Some(
            "DocuGraph MCP: High-performance Knowledge Graph & Evidence Retrieval for complex technical PDFs with strict provenance and token budgeting."
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
    /// Create a new server instance with the auto-generated tool router and disk-backed store.
    pub fn new() -> Self {
        let cache = DiskCache::new(DiskCache::default_dir()).ok();
        Self {
            tool_router: Self::tool_router(),
            store: DocumentStore::new(cache),
        }
    }

    /// Create a server with a custom DocumentStore (useful for testing).
    pub fn with_store(store: DocumentStore) -> Self {
        Self {
            tool_router: Self::tool_router(),
            store,
        }
    }

    /// Register or update a document in the server store.
    pub async fn register_document(&self, doc: Document) {
        if let Err(e) = self.store.insert(doc) {
            tracing::error!("Failed to register document in store: {}", e);
        }
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

    /// Helper to fetch a target document or all loaded documents.
    fn get_documents(&self, doc_id: Option<&str>) -> Vec<Document> {
        if let Some(doc) = doc_id.and_then(|id| self.store.get(id)) {
            return vec![doc];
        }
        let metas = self.store.list_documents();
        metas
            .into_iter()
            .filter_map(|m| self.store.get(&m.id))
            .collect()
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

    /// List all indexed documents available in the knowledge base.
    #[tool(
        name = "document_list",
        description = "List all indexed PDF documents currently available with page counts and SHA-256 hashes."
    )]
    pub async fn document_list(&self) -> String {
        let metas = self.store.list_documents();
        let list: Vec<DocumentSummary> = metas
            .into_iter()
            .map(|m| DocumentSummary {
                id: m.id,
                title: m.title,
                total_pages: m.total_pages,
                total_sections: m.total_sections,
                content_hash: m.content_hash,
                indexed_at: m.indexed_at,
            })
            .collect();
        serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_string())
    }

    /// Retrieve high-level metadata and section preview for a specific document.
    #[tool(
        name = "document_info",
        description = "Get structural metadata, page count, and section overview for an indexed document."
    )]
    pub async fn document_info(&self, params: Parameters<DocumentInfoParams>) -> String {
        let doc_id = &params.0.document_id;
        if let Some(doc) = self.store.get(doc_id) {
            let preview: Vec<String> = doc
                .sections
                .iter()
                .flat_map(|s| s.flatten())
                .take(25)
                .map(|s| {
                    let indent = "  ".repeat(s.level.saturating_sub(1) as usize);
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
                content_hash: doc.metadata.content_hash.clone(),
                sections_preview: preview,
            };
            serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string())
        } else {
            serde_json::to_string_pretty(&DocumentInfoResult {
                id: doc_id.clone(),
                title: format!("Document Not Found: {}", doc_id),
                total_pages: 0,
                total_sections: 0,
                content_hash: String::new(),
                sections_preview: vec![
                    "Document not found. Use 'document_list' to view available documents."
                        .to_string(),
                ],
            })
            .unwrap_or_else(|_| "{}".to_string())
        }
    }

    /// Retrieve the hierarchical table of contents (outline) of a document with page ranges.
    #[tool(
        name = "document_outline",
        description = "Get the hierarchical outline tree (H1, H2, H3) with exact page ranges and section IDs."
    )]
    pub async fn document_outline(&self, params: Parameters<DocumentOutlineParams>) -> String {
        let doc_id = &params.0.document_id;
        let max_depth = params.0.max_depth.unwrap_or(3);

        if let Some(doc) = self.store.get(doc_id) {
            let tree: Vec<OutlineNodeResult> = doc
                .sections
                .iter()
                .filter_map(|s| map_outline_node(s, 1, max_depth))
                .collect();
            serde_json::to_string_pretty(&tree).unwrap_or_else(|_| "[]".to_string())
        } else {
            format!("Error: Document '{doc_id}' not found.")
        }
    }

    /// Perform fast Okapi BM25 keyword search over document sections and pages.
    #[tool(
        name = "document_search",
        description = "Fast BM25 keyword search across sections and pages. Returns ranked snippets with citations."
    )]
    pub async fn document_search(&self, params: Parameters<DocumentSearchParams>) -> String {
        let limit = params.0.limit.unwrap_or(5);
        let docs = self.get_documents(params.0.document_id.as_deref());
        if docs.is_empty() {
            return "No documents available for search.".to_string();
        }

        let bm25 = crate::retrieval::Bm25Index::build_from_documents(&docs, None);
        let hits = bm25.search(&params.0.query, limit);
        serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".to_string())
    }

    /// Perform hybrid search (BM25 + Semantic Cosine + Structural Boost) with configurable weights.
    #[tool(
        name = "document_search_hybrid",
        description = "Hybrid search combining keywords (BM25), conceptual semantic similarity, and structural boosts."
    )]
    pub async fn document_search_hybrid(
        &self,
        params: Parameters<DocumentSearchHybridParams>,
    ) -> String {
        let limit = params.0.limit.unwrap_or(5);
        let docs = self.get_documents(params.0.document_id.as_deref());
        if docs.is_empty() {
            return "No documents available for search.".to_string();
        }

        let weights = HybridWeights {
            bm25_weight: params.0.bm25_weight.unwrap_or(0.50),
            semantic_weight: params.0.semantic_weight.unwrap_or(0.30),
            structural_weight: params.0.structural_weight.unwrap_or(0.20),
        };

        let retriever = HybridRetriever::build(&docs, None, Some(weights));
        let hits = retriever.search(&params.0.query, limit);
        serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".to_string())
    }

    /// Retrieve the full content of a specific section with optional parent context and token budgeting.
    #[tool(
        name = "document_get_section",
        description = "Fetch the text and sub-clauses of a specific section by ID with token budgeting."
    )]
    pub async fn document_get_section(
        &self,
        params: Parameters<DocumentGetSectionParams>,
    ) -> String {
        let doc_id = &params.0.document_id;
        let section_id = &params.0.section_id;
        let include_parent = params.0.include_parent.unwrap_or(true);
        let budget = ContextBudget {
            max_tokens: params.0.max_tokens.unwrap_or(1500),
            max_chunks: 10,
            compact: true,
        };

        if let Some(doc) = self.store.get(doc_id) {
            match ContextBuilder::expand_section_context(&doc, section_id, include_parent, budget) {
                Some(content) => content,
                None => format!("Error: Section '{section_id}' not found in document '{doc_id}'."),
            }
        } else {
            format!("Error: Document '{doc_id}' not found.")
        }
    }

    /// Retrieve compact evidence snippets with guaranteed citation provenance for LLM reasoning.
    #[tool(
        name = "document_get_evidence",
        description = "Gather compact, verifiable evidence snippets with exact page and section citations for LLM reasoning."
    )]
    pub async fn document_get_evidence(
        &self,
        params: Parameters<DocumentGetEvidenceParams>,
    ) -> String {
        let query = &params.0.query;
        let docs = self.get_documents(params.0.document_id.as_deref());
        if docs.is_empty() {
            return "No documents available for evidence collection.".to_string();
        }

        let budget = ContextBudget {
            max_tokens: params.0.max_tokens.unwrap_or(1200),
            max_chunks: params.0.max_items.unwrap_or(4),
            compact: true,
        };

        let retriever = HybridRetriever::build(&docs, None, None);
        let hits = retriever.search(query, budget.max_chunks * 2);
        let bundle = ContextBuilder::build_evidence(query, &hits, budget);
        bundle.to_markdown()
    }

    /// Read raw text from a specific page range with a character budget.
    #[tool(
        name = "document_read_pages",
        description = "Read sequential pages directly from a document with strict character bounds."
    )]
    pub async fn document_read_pages(&self, params: Parameters<DocumentReadPagesParams>) -> String {
        let doc_id = &params.0.document_id;
        let max_chars = params.0.max_chars.unwrap_or(8000);

        if let Some(doc) = self.store.get(doc_id) {
            let mut out = String::new();
            out.push_str(&format!(
                "# Lectura de Páginas: {} (pp. {}-{})\n\n",
                doc.metadata.title, params.0.page_start, params.0.page_end
            ));

            let mut chars_count = 0;
            for p in params.0.page_start..=params.0.page_end {
                if let Some(page) = doc.get_page(p) {
                    let page_header = format!("--- Página {} ---\n", p);
                    if chars_count + page_header.len() + page.text.len() > max_chars {
                        let remaining = max_chars.saturating_sub(chars_count + page_header.len());
                        out.push_str(&page_header);
                        out.push_str(&page.text.chars().take(remaining).collect::<String>());
                        out.push_str("\n\n*(Límite de caracteres alcanzado)*\n");
                        break;
                    }
                    out.push_str(&page_header);
                    out.push_str(&page.text);
                    out.push_str("\n\n");
                    chars_count += page_header.len() + page.text.len();
                }
            }
            out
        } else {
            format!("Error: Document '{doc_id}' not found.")
        }
    }

    /// Retrieve a design pattern (Intent, Motivation, Structure, Participants, Consequences) dynamically from the document.
    #[tool(
        name = "pattern_get",
        description = "Dynamically extract Design Pattern components (Intent, Motivation, Participants, Consequences, Sample Code) from the indexed literature."
    )]
    pub async fn pattern_get(&self, params: Parameters<PatternGetParams>) -> String {
        let docs = self.get_documents(params.0.document_id.as_deref());
        for doc in &docs {
            if let Some(pattern) = DesignPatternsAdapter::get_pattern(doc, &params.0.pattern_name) {
                return serde_json::to_string_pretty(&pattern).unwrap_or_else(|_| "{}".to_string());
            }
        }
        format!(
            "Pattern '{}' not found in available documents.",
            params.0.pattern_name
        )
    }

    /// Compare two design patterns dynamically using extracted evidence from the document.
    #[tool(
        name = "pattern_compare",
        description = "Compare two design patterns side-by-side based on their extracted intents, applicability, and consequences."
    )]
    pub async fn pattern_compare(&self, params: Parameters<PatternCompareParams>) -> String {
        let docs = self.get_documents(params.0.document_id.as_deref());
        for doc in &docs {
            if let Some((a, b)) = DesignPatternsAdapter::compare_patterns(
                doc,
                &params.0.pattern_a,
                &params.0.pattern_b,
            ) {
                let comparison = serde_json::json!({
                    "pattern_a": a,
                    "pattern_b": b,
                    "summary": format!("Comparison of {} vs {} from '{}'", a.name, b.name, doc.metadata.title)
                });
                return serde_json::to_string_pretty(&comparison)
                    .unwrap_or_else(|_| "{}".to_string());
            }
        }
        format!(
            "Could not find both '{}' and '{}' for comparison.",
            params.0.pattern_a, params.0.pattern_b
        )
    }
}

fn map_outline_node(
    node: &SectionNode,
    current_depth: u32,
    max_depth: u32,
) -> Option<OutlineNodeResult> {
    if current_depth > max_depth {
        return None;
    }

    let children = if current_depth < max_depth {
        node.children
            .iter()
            .filter_map(|c| map_outline_node(c, current_depth + 1, max_depth))
            .collect()
    } else {
        Vec::new()
    };

    Some(OutlineNodeResult {
        id: node.id.clone(),
        title: node.title.clone(),
        level: node.level,
        page_start: node.page_start,
        page_end: node.page_end,
        children,
    })
}
