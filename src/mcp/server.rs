//! DocuGraph Model Context Protocol (MCP) server implementation.

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use std::path::Path;
use tracing::info;

use super::error::{ToolError, ToolResult};
use super::tools::*;
use crate::document::model::{Document, PageKind, SectionNode};
use crate::multimodal::CachedPageRendererProxy;
use crate::retrieval::{ContextBudget, ContextBuilder, HybridRetriever, HybridWeights};
use crate::storage::{DiskCache, DocumentStore};

/// Maximum number of document ids or attachment names quoted in a "not found" error.
const MAX_LISTED_NAMES: usize = 20;

/// DocuGraph MCP server holding the tool router and shared document store.
#[derive(Clone)]
pub struct DocuGraphServer {
    tool_router: ToolRouter<Self>,
    store: DocumentStore,
    renderer: CachedPageRendererProxy,
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
    ///
    /// The cache directory comes from `DOCUGRAPH_CACHE_DIR` (see [`DiskCache::default_dir`]).
    pub fn new() -> Self {
        Self::with_cache_dir(DiskCache::default_dir())
    }

    /// Create a server whose document store and page renders live in `cache_dir`.
    pub fn with_cache_dir(cache_dir: impl AsRef<Path>) -> Self {
        let cache_dir = cache_dir.as_ref();
        let cache = DiskCache::new(cache_dir).ok();
        let renderer = CachedPageRendererProxy::new(Some(cache_dir));
        Self {
            tool_router: Self::tool_router(),
            store: DocumentStore::new(cache),
            renderer,
        }
    }

    /// Create a server with a custom DocumentStore (useful for testing).
    pub fn with_store(store: DocumentStore) -> Self {
        let renderer = CachedPageRendererProxy::new(None::<&std::path::Path>);
        Self {
            tool_router: Self::tool_router(),
            store,
            renderer,
        }
    }

    /// Create a server with custom DocumentStore and renderer.
    pub fn with_store_and_renderer(
        store: DocumentStore,
        renderer: CachedPageRendererProxy,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            store,
            renderer,
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

    /// Documents to search: the one named by `doc_id`, or every indexed document.
    ///
    /// An unknown `doc_id` is an error listing the available ids. It used to fall back to
    /// searching every document, which hid typos behind results from the wrong book.
    fn get_documents(&self, doc_id: Option<&str>) -> Result<Vec<Document>, ToolError> {
        if let Some(id) = doc_id {
            return self.require_document(id).map(|doc| vec![doc]);
        }
        let metas = self.store.list_documents();
        Ok(metas
            .into_iter()
            .filter_map(|m| self.store.get(&m.id))
            .collect())
    }

    /// Fetch a document by id or content hash, or explain which ids exist.
    fn require_document(&self, doc_id: &str) -> Result<Document, ToolError> {
        self.store
            .get(doc_id)
            .ok_or_else(|| self.document_not_found(doc_id))
    }

    /// Error for an unknown document id that lists the ids the agent can use instead.
    fn document_not_found(&self, doc_id: &str) -> ToolError {
        let mut ids: Vec<String> = self
            .store
            .list_documents()
            .into_iter()
            .map(|m| m.id)
            .collect();
        if ids.is_empty() {
            return ToolError::new(format!(
                "Document '{doc_id}' not found: no documents are indexed."
            ));
        }
        ids.sort();
        ToolError::new(format!(
            "Document '{doc_id}' not found. Available document ids: {}.",
            quoted_list(&ids)
        ))
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
                is_encrypted: m.is_encrypted,
                untrusted_text_detected: m.untrusted_text_detected,
                scanned_pages_count: m.scanned_pages_count,
            })
            .collect();
        serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_string())
    }

    /// Retrieve high-level metadata and section preview for a specific document.
    #[tool(
        name = "document_info",
        description = "Get structural metadata, page count, and section overview for an indexed document."
    )]
    pub async fn document_info(&self, params: Parameters<DocumentInfoParams>) -> ToolResult {
        let doc = self.require_document(&params.0.document_id)?;
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

        let scan_warning = if doc.metadata.scanned_pages_count > 0 {
            Some(format!(
                "⚠️ {} de {} páginas parecen ser imágenes escaneadas sin capa de texto digital. Se recomienda OCR externo.",
                doc.metadata.scanned_pages_count, doc.metadata.total_pages
            ))
        } else {
            None
        };

        let info = DocumentInfoResult {
            id: doc.id.to_string(),
            title: doc.metadata.title.clone(),
            total_pages: doc.metadata.total_pages,
            total_sections: doc.total_sections() as u32,
            content_hash: doc.metadata.content_hash.clone(),
            sections_preview: preview,
            is_encrypted: doc.metadata.is_encrypted,
            untrusted_text_detected: doc.metadata.untrusted_text_detected,
            scanned_pages_count: doc.metadata.scanned_pages_count,
            scan_warning,
            total_links: doc.metadata.total_links,
            has_forms: doc.metadata.has_forms,
            total_form_fields: doc.metadata.total_form_fields,
            is_tagged: doc.metadata.is_tagged,
            has_attachments: doc.metadata.has_attachments,
            total_attachments: doc.metadata.total_attachments,
        };
        Ok(serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".to_string()))
    }

    /// Retrieve the hierarchical table of contents (outline) of a document with page ranges.
    #[tool(
        name = "document_outline",
        description = "Get the hierarchical outline tree (H1, H2, H3) with exact page ranges and section IDs."
    )]
    pub async fn document_outline(&self, params: Parameters<DocumentOutlineParams>) -> ToolResult {
        let doc = self.require_document(&params.0.document_id)?;
        let max_depth = params.0.max_depth.unwrap_or(3);

        let tree: Vec<OutlineNodeResult> = doc
            .sections
            .iter()
            .filter_map(|s| map_outline_node(s, 1, max_depth))
            .collect();
        Ok(serde_json::to_string_pretty(&tree).unwrap_or_else(|_| "[]".to_string()))
    }

    /// Perform fast Okapi BM25 keyword search over document sections and pages.
    #[tool(
        name = "document_search",
        description = "Fast BM25 keyword search across sections and pages. Returns ranked snippets with citations."
    )]
    pub async fn document_search(&self, params: Parameters<DocumentSearchParams>) -> ToolResult {
        let limit = bounded(params.0.limit, 5, MAX_SEARCH_LIMIT);
        let docs = self.get_documents(params.0.document_id.as_deref())?;
        if docs.is_empty() {
            return Ok("No documents available for search.".to_string());
        }

        let bm25 = crate::retrieval::Bm25Index::build_from_documents(&docs, None);
        let hits = bm25.search(&params.0.query, limit);
        Ok(serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".to_string()))
    }

    /// Perform hybrid search (BM25 + Semantic Cosine + Structural Boost) with configurable weights.
    #[tool(
        name = "document_search_hybrid",
        description = "Hybrid search combining keywords (BM25), conceptual semantic similarity, and structural boosts."
    )]
    pub async fn document_search_hybrid(
        &self,
        params: Parameters<DocumentSearchHybridParams>,
    ) -> ToolResult {
        let limit = bounded(params.0.limit, 5, MAX_SEARCH_LIMIT);
        let docs = self.get_documents(params.0.document_id.as_deref())?;
        if docs.is_empty() {
            return Ok("No documents available for search.".to_string());
        }

        let weights = HybridWeights {
            bm25_weight: params.0.bm25_weight.unwrap_or(0.50),
            semantic_weight: params.0.semantic_weight.unwrap_or(0.30),
            structural_weight: params.0.structural_weight.unwrap_or(0.20),
        };

        let retriever = HybridRetriever::build(&docs, None, Some(weights));
        let hits = retriever.search(&params.0.query, limit);
        Ok(serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".to_string()))
    }

    /// Retrieve the full content of a specific section with optional parent context and token budgeting.
    #[tool(
        name = "document_get_section",
        description = "Fetch the text and sub-clauses of a specific section by ID with token budgeting."
    )]
    pub async fn document_get_section(
        &self,
        params: Parameters<DocumentGetSectionParams>,
    ) -> ToolResult {
        let doc_id = &params.0.document_id;
        let section_id = &params.0.section_id;
        let include_parent = params.0.include_parent.unwrap_or(true);
        let budget = ContextBudget {
            max_tokens: bounded(params.0.max_tokens, 1500, MAX_CONTEXT_TOKENS),
            max_chunks: 10,
            compact: true,
        };

        let doc = self.require_document(doc_id)?;
        ContextBuilder::expand_section_context(&doc, section_id, include_parent, budget).ok_or_else(
            || {
                ToolError::new(format!(
                    "Section '{section_id}' not found in document '{doc_id}'. Use document_outline to list section ids."
                ))
            },
        )
    }

    /// Retrieve broader surrounding conceptual context for a topic or query across the document graph.
    #[tool(
        name = "document_get_context",
        description = "Retrieve surrounding conceptual context (parent headings, sub-clauses, and related paragraphs) for a query or topic within a token budget."
    )]
    pub async fn document_get_context(
        &self,
        params: Parameters<DocumentGetContextParams>,
    ) -> ToolResult {
        let query = &params.0.query;
        let docs = self.get_documents(params.0.document_id.as_deref())?;
        if docs.is_empty() {
            return Ok("No documents available for context expansion.".to_string());
        }

        let budget = ContextBudget {
            max_tokens: bounded(params.0.max_tokens, 1500, MAX_CONTEXT_TOKENS),
            max_chunks: bounded(params.0.max_chunks, 5, MAX_CONTEXT_CHUNKS),
            compact: true,
        };

        let retriever = HybridRetriever::build(&docs, None, None);
        let hits = retriever.search(query, budget.max_chunks.saturating_mul(2));
        Ok(ContextBuilder::build_conceptual_context(
            query, &hits, &docs, budget,
        ))
    }

    /// Retrieve compact evidence snippets with guaranteed citation provenance for LLM reasoning.
    #[tool(
        name = "document_get_evidence",
        description = "Gather compact, verifiable evidence snippets with exact page and section citations for LLM reasoning."
    )]
    pub async fn document_get_evidence(
        &self,
        params: Parameters<DocumentGetEvidenceParams>,
    ) -> ToolResult {
        let query = &params.0.query;
        let docs = self.get_documents(params.0.document_id.as_deref())?;
        if docs.is_empty() {
            return Ok("No documents available for evidence collection.".to_string());
        }

        let budget = ContextBudget {
            max_tokens: bounded(params.0.max_tokens, 1200, MAX_CONTEXT_TOKENS),
            max_chunks: bounded(params.0.max_items, 4, MAX_CONTEXT_CHUNKS),
            compact: true,
        };

        let retriever = HybridRetriever::build(&docs, None, None);
        let hits = retriever.search(query, budget.max_chunks.saturating_mul(2));
        let bundle = ContextBuilder::build_evidence(query, &hits, budget);
        Ok(bundle.to_markdown())
    }

    /// Read raw text from a specific page range with a character budget.
    #[tool(
        name = "document_read_pages",
        description = "Read sequential pages directly from a document with strict character bounds."
    )]
    pub async fn document_read_pages(
        &self,
        params: Parameters<DocumentReadPagesParams>,
    ) -> ToolResult {
        let DocumentReadPagesParams {
            document_id,
            page_start,
            page_end,
            max_chars,
        } = params.0;
        let max_chars = bounded(max_chars, 8000, MAX_READ_CHARS);
        let doc = self.require_document(&document_id)?;

        if page_start == 0 {
            return Err(ToolError::new(
                "page_start must be at least 1: pages are numbered from 1.",
            ));
        }
        if page_start > page_end {
            return Err(ToolError::new(format!(
                "Invalid page range {page_start}-{page_end}: page_start must not be greater than page_end."
            )));
        }
        let total = page_count(&doc);
        if page_start > total {
            return Err(ToolError::new(format!(
                "Page range {page_start}-{page_end} is empty: document '{}' has {total} pages.",
                doc.id
            )));
        }
        // Clamp the end and walk only the pages that exist, so an open-ended range such as
        // page_end = u32::MAX costs the same as reading up to the last page.
        let last_page = page_end.min(total);
        let pages = doc.pages_in_range(page_start, last_page);
        if pages.is_empty() {
            return Err(ToolError::new(format!(
                "Page range {page_start}-{last_page} is empty: document '{}' has no extracted pages in it.",
                doc.id
            )));
        }

        let mut out = String::new();
        out.push_str(&format!(
            "# Lectura de Páginas: {} (pp. {}-{})\n\n",
            doc.metadata.title, page_start, last_page
        ));
        if last_page < page_end {
            out.push_str(&format!(
                "*(page_end {page_end} ajustado a la última página del documento: {last_page})*\n\n"
            ));
        }

        let mut chars_count = 0;
        for page in pages {
            let p = page.page_number;
            let page_header = if page.kind == PageKind::ScannedImage {
                format!(
                    "--- Página {} [📷 Imagen Escaneada / Sin Capa de Texto] ---\n",
                    p
                )
            } else if page.untrusted_text_detected {
                format!("--- Página {} [⚠️ Untrusted Hidden Text Detected] ---\n", p)
            } else if page.kind == PageKind::Empty {
                format!("--- Página {} [Página en Blanco] ---\n", p)
            } else {
                format!("--- Página {} ---\n", p)
            };
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
        Ok(out)
    }

    /// Render a specific document page to a high-resolution PNG image for visual multimodal inspection.
    #[tool(
        name = "document_render_page",
        description = "Render a specific document page to a high-resolution PNG image for visual inspection (diagrams, complex charts, scans) by Multimodal LLMs."
    )]
    pub async fn document_render_page(&self, params: Parameters<RenderPageParams>) -> ToolResult {
        let doc_id = &params.0.document_id;
        let page_num = params.0.page_number;
        let max_width = params.0.max_width.unwrap_or(1024);

        let doc = self.require_document(doc_id)?;
        check_page(&doc, page_num)?;
        let rendered = self
            .renderer
            .render_document_page(&doc, page_num, max_width)
            .map_err(|e| {
                ToolError::new(format!(
                    "Failed to render page {page_num} of document '{doc_id}': {e}"
                ))
            })?;

        let data_uri = format!("data:image/png;base64,{}", rendered.base64_data);
        let res = RenderPageResult {
            document_id: doc_id.clone(),
            page_number: rendered.page_number,
            width: rendered.width,
            height: rendered.height,
            mime_type: "image/png".to_string(),
            base64_image: rendered.base64_data,
            data_uri,
            from_cache: rendered.from_cache,
        };
        Ok(serde_json::to_string_pretty(&res).unwrap_or_else(|_| "{}".to_string()))
    }

    /// Extract hyperlinks and internal cross-references from a document.
    #[tool(
        name = "document_get_links",
        description = "Extract hyperlinks and internal cross-references from a document with exact page numbers, URLs, and coordinates."
    )]
    pub async fn document_get_links(
        &self,
        params: Parameters<DocumentGetLinksParams>,
    ) -> ToolResult {
        let doc_id = &params.0.document_id;
        let page_filter = params.0.page;
        let kind_filter = params.0.kind.as_deref().unwrap_or("all").to_lowercase();

        let doc = self.require_document(doc_id)?;
        if let Some(p) = page_filter {
            check_page(&doc, p)?;
        }

        let mut results = Vec::new();

        let target_pages: Vec<&crate::document::Page> = if let Some(p) = page_filter {
            doc.get_page(p).into_iter().collect()
        } else {
            doc.pages.iter().collect()
        };

        for page in target_pages {
            for link in &page.links {
                let kind_str = match &link.target {
                    crate::document::LinkTarget::Uri(_) => "external",
                    crate::document::LinkTarget::InternalPage(_) => "internal",
                    crate::document::LinkTarget::Named(_) => "named",
                };

                let matches_kind = match kind_filter.as_str() {
                    "external" => link.is_external(),
                    "internal" => link.is_internal(),
                    _ => true,
                };

                if matches_kind {
                    results.push(DocumentLinkResult {
                        page_number: link.page_number,
                        kind: kind_str.to_string(),
                        uri: link.uri.clone(),
                        target_page: link.target_page,
                        named_target: match &link.target {
                            crate::document::LinkTarget::Named(name) => Some(name.clone()),
                            _ => None,
                        },
                        rect: link.rect,
                    });
                }
            }
        }

        let response = DocumentGetLinksResult {
            document_id: doc_id.clone(),
            total_links: results.len(),
            links: results,
        };

        Ok(serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string()))
    }

    /// Extract interactive form fields (AcroForms) from a document.
    #[tool(
        name = "document_get_forms",
        description = "Extract interactive AcroForm fields (text inputs, checkboxes, radio buttons, dropdowns) with names, values, and page coordinates."
    )]
    pub async fn document_get_forms(
        &self,
        params: Parameters<DocumentGetFormsParams>,
    ) -> ToolResult {
        let doc_id = &params.0.document_id;
        let page_filter = params.0.page;
        let filled_only = params.0.filled_only.unwrap_or(false);

        let doc = self.require_document(doc_id)?;
        if let Some(p) = page_filter {
            check_page(&doc, p)?;
        }

        let mut results = Vec::new();

        for field in &doc.forms {
            if page_filter.is_some_and(|target_p| field.page_number != Some(target_p)) {
                continue;
            }

            if filled_only
                && field
                    .value
                    .as_deref()
                    .map(|v| v.trim().is_empty())
                    .unwrap_or(true)
            {
                continue;
            }

            let type_str = field.field_type.as_str();

            results.push(FormFieldResult {
                name: field.name.clone(),
                fully_qualified_name: field.fully_qualified_name.clone(),
                field_type: type_str.to_string(),
                value: field.value.clone(),
                default_value: field.default_value.clone(),
                read_only: field.read_only,
                required: field.required,
                page_number: field.page_number,
                rect: field.rect,
            });
        }

        let response = DocumentGetFormsResult {
            document_id: doc_id.clone(),
            total_fields: results.len(),
            fields: results,
        };

        Ok(serde_json::to_string_pretty(&response).unwrap_or_else(|_| "{}".to_string()))
    }

    /// Retrieve metadata for all embedded files and attachments inside a document.
    #[tool(
        name = "document_get_attachments",
        description = "List all embedded file attachments (e.g. ZUGFeRD/Factur-X XML, CSV, datasets) with names, MIME types, and sizes."
    )]
    pub async fn document_get_attachments(
        &self,
        params: Parameters<DocumentGetAttachmentsParams>,
    ) -> ToolResult {
        let doc_id = &params.0.document_id;
        let doc = self.require_document(doc_id)?;
        let attachments: Vec<AttachmentSummaryResult> = doc
            .attachments
            .iter()
            .map(|att| AttachmentSummaryResult {
                id: att.id.clone(),
                filename: att.filename.clone(),
                description: att.description.clone(),
                mime_type: att.mime_type.clone(),
                size_bytes: att.size_bytes,
                checksum_md5: att.checksum_md5.clone(),
                mod_date: att.mod_date.clone(),
                is_text: att.is_text,
                page_number: att.page_number,
            })
            .collect();

        let res = DocumentGetAttachmentsResult {
            document_id: doc_id.clone(),
            total_attachments: attachments.len(),
            attachments,
        };
        Ok(serde_json::to_string_pretty(&res).unwrap_or_else(|_| "{}".to_string()))
    }

    /// Read and decode the content of an embedded file attachment (e.g. ZUGFeRD XML, CSV, dataset).
    #[tool(
        name = "document_read_attachment",
        description = "Read and decode the content of an embedded attachment by filename or identifier. Returns text (UTF-8) or base64."
    )]
    pub async fn document_read_attachment(
        &self,
        params: Parameters<DocumentReadAttachmentParams>,
    ) -> ToolResult {
        let doc_id = &params.0.document_id;
        let name_or_id = &params.0.name_or_id;
        let max_bytes = bounded(params.0.max_bytes, 524_288, MAX_ATTACHMENT_BYTES);

        let doc = self.require_document(doc_id)?;
        let att = doc
            .get_attachment(name_or_id)
            .ok_or_else(|| attachment_not_found(&doc, name_or_id))?;

        let force_base64 = params.0.encoding.as_deref() == Some("base64");
        let should_be_text = !force_base64 && att.is_text;

        let (encoded_content, truncated) = if att.data.len() > max_bytes {
            let slice = &att.data[..max_bytes];
            if should_be_text {
                let text = String::from_utf8_lossy(slice).to_string();
                (text, true)
            } else {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(slice);
                (b64, true)
            }
        } else if should_be_text {
            let text = String::from_utf8_lossy(&att.data).to_string();
            (text, false)
        } else {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&att.data);
            (b64, false)
        };

        let res = DocumentReadAttachmentResult {
            document_id: doc_id.clone(),
            filename: att.filename.clone(),
            mime_type: att.mime_type.clone(),
            size_bytes: att.size_bytes,
            encoding: if should_be_text {
                "text".to_string()
            } else {
                "base64".to_string()
            },
            content: encoded_content,
            truncated,
        };

        Ok(serde_json::to_string_pretty(&res).unwrap_or_else(|_| "{}".to_string()))
    }
}

/// Highest page number of a document, taken from its metadata or its extracted pages.
fn page_count(doc: &Document) -> u32 {
    doc.pages
        .iter()
        .map(|p| p.page_number)
        .max()
        .unwrap_or(0)
        .max(doc.metadata.total_pages)
}

/// Error unless `page` is a valid 1-based page number of `doc`.
fn check_page(doc: &Document, page: u32) -> Result<(), ToolError> {
    let total = page_count(doc);
    if total == 0 {
        return Err(ToolError::new(format!(
            "Page {page} does not exist: document '{}' has no pages.",
            doc.id
        )));
    }
    if page == 0 || page > total {
        return Err(ToolError::new(format!(
            "Page {page} does not exist in document '{}': valid pages are 1-{total}.",
            doc.id
        )));
    }
    Ok(())
}

/// Error for an unknown attachment that lists the attachments the document does have.
fn attachment_not_found(doc: &Document, name_or_id: &str) -> ToolError {
    if doc.attachments.is_empty() {
        return ToolError::new(format!(
            "Attachment '{name_or_id}' not found: document '{}' has no attachments.",
            doc.id
        ));
    }
    let names: Vec<String> = doc.attachments.iter().map(|a| a.filename.clone()).collect();
    ToolError::new(format!(
        "Attachment '{name_or_id}' not found in document '{}'. Available attachments: {}.",
        doc.id,
        quoted_list(&names)
    ))
}

/// Quote names for an error message, keeping at most [`MAX_LISTED_NAMES`] of them.
fn quoted_list(names: &[String]) -> String {
    let mut listed: Vec<String> = names
        .iter()
        .take(MAX_LISTED_NAMES)
        .map(|name| format!("'{name}'"))
        .collect();
    if names.len() > MAX_LISTED_NAMES {
        listed.push(format!("... ({} more)", names.len() - MAX_LISTED_NAMES));
    }
    listed.join(", ")
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
