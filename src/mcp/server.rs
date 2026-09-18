//! DocuGraph Model Context Protocol (MCP) server implementation.

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use tracing::info;

use super::tools::*;
use crate::document::model::{Document, PageKind, SectionNode};
use crate::multimodal::CachedPageRendererProxy;
use crate::retrieval::{ContextBudget, ContextBuilder, HybridWeights, RetrieverCache};
use crate::storage::{DiskCache, DocumentStore};

/// DocuGraph MCP server holding the tool router and shared document store.
#[derive(Clone)]
pub struct DocuGraphServer {
    tool_router: ToolRouter<Self>,
    store: DocumentStore,
    renderer: CachedPageRendererProxy,
    retrievers: RetrieverCache,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DocuGraphServer {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
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
        let cache_dir = DiskCache::default_dir();
        let cache = DiskCache::new(&cache_dir).ok();
        let renderer = CachedPageRendererProxy::new(Some(&cache_dir));
        Self {
            tool_router: Self::tool_router(),
            store: DocumentStore::new(cache),
            renderer,
            retrievers: RetrieverCache::default(),
        }
    }

    /// Create a server with a custom DocumentStore (useful for testing).
    pub fn with_store(store: DocumentStore) -> Self {
        let renderer = CachedPageRendererProxy::new(None::<&std::path::Path>);
        Self {
            tool_router: Self::tool_router(),
            store,
            renderer,
            retrievers: RetrieverCache::default(),
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
            retrievers: RetrieverCache::default(),
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

    /// Resolve the search scope: the named document, or every indexed document
    /// when no id is given.
    ///
    /// An explicit id that does not resolve is an error, never a silent widening to
    /// the whole corpus: a typo must not return cited passages from another document.
    fn resolve_scope(&self, doc_id: Option<&str>) -> Result<Vec<Document>, String> {
        match doc_id.map(str::trim).filter(|id| !id.is_empty()) {
            Some(id) => self.require_document(id).map(|doc| vec![doc]),
            None => {
                let docs: Vec<Document> = self
                    .store
                    .list_documents()
                    .into_iter()
                    .filter_map(|m| self.store.get(&m.id))
                    .collect();
                if docs.is_empty() {
                    Err(Self::EMPTY_CORPUS.to_string())
                } else {
                    Ok(docs)
                }
            }
        }
    }

    const EMPTY_CORPUS: &'static str =
        "No documents indexed. Run `docugraph index <path.pdf>` first.";

    /// Resolve a `document_id`, or explain once why it did not resolve.
    ///
    /// The single place a tool turns an id into a `Document`. The message naming
    /// the ids an agent can actually use is written once and every tool inherits
    /// it, instead of nine handlers each saying "not found" and stopping there.
    fn require_document(&self, doc_id: &str) -> Result<Document, String> {
        let id = doc_id.trim();
        self.store
            .get(id)
            .ok_or_else(|| self.unknown_document_error(id))
    }

    /// Name the ids the agent can actually use, so an unknown id is correctable
    /// in one follow-up call instead of being guessed at.
    fn unknown_document_error(&self, id: &str) -> String {
        let available: Vec<String> = self
            .store
            .list_documents()
            .into_iter()
            .take(10)
            .map(|m| m.id)
            .collect();
        if available.is_empty() {
            return format!("Document '{id}' not found. {}", Self::EMPTY_CORPUS);
        }
        format!(
            "Document '{id}' not found. Available: {}. Use 'document_list' for the full list.",
            available.join(", ")
        )
    }
}

#[tool_router(router = tool_router)]
impl DocuGraphServer {
    /// List all indexed documents available in the knowledge base.
    #[tool(
        name = "document_list",
        description = "List the indexed PDF documents, with page counts, SHA-256 hashes, and the cache directory they were read from. Call this first to learn the document_id values the other tools take."
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

        let cache_dir = self
            .store
            .cache_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "(in-memory only)".to_string());

        let hint = list.is_empty().then(|| {
            format!(
                "No documents found in {cache_dir}. Index one with `docugraph index <path.pdf>`, \
                 and make sure it writes to this same directory - set DOCUGRAPH_CACHE_DIR to it in \
                 both the indexing shell and this server's MCP configuration, since they are \
                 separate processes."
            )
        });

        let result = DocumentListResult {
            cache_dir,
            total: list.len(),
            documents: list,
            hint,
        };
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".to_string())
    }

    /// Retrieve high-level metadata and section preview for a specific document.
    #[tool(
        name = "document_info",
        description = "Get structural metadata, page count, and section overview for an indexed document."
    )]
    pub async fn document_info(
        &self,
        params: Parameters<DocumentInfoParams>,
    ) -> Result<String, String> {
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
    pub async fn document_outline(
        &self,
        params: Parameters<DocumentOutlineParams>,
    ) -> Result<String, String> {
        let max_depth = params.0.max_depth.unwrap_or(3);
        let doc = self.require_document(&params.0.document_id)?;

        let tree: Vec<OutlineNodeResult> = doc
            .sections
            .iter()
            .filter_map(|s| map_outline_node(s, 1, max_depth))
            .collect();
        Ok(serde_json::to_string_pretty(&tree).unwrap_or_else(|_| "[]".to_string()))
    }

    /// Answer a question from the indexed documents, returning only what carries it.
    #[tool(
        name = "document_query",
        description = "Answer a question from the indexed PDFs with exact page citations, returning only the passages that carry it. Answers \"no evidence\" when the corpus does not cover the question, rather than the closest passages it has."
    )]
    pub async fn document_query(
        &self,
        params: Parameters<DocumentQueryParams>,
    ) -> Result<String, String> {
        let query = &params.0.query;
        let docs = self.resolve_scope(params.0.document_id.as_deref())?;
        let mode = params.0.mode.unwrap_or_default();
        let max_items = params.0.max_items.unwrap_or(4);

        let budget = ContextBudget {
            max_tokens: params.0.max_tokens.unwrap_or(1200),
            max_chunks: max_items,
            compact: true,
        };

        // The budgeted modes drop candidates while packing, so they ask for more
        // than they will keep; `hits` returns the ranking itself and asks for what
        // it was told to return.
        let wanted = match mode {
            QueryMode::Hits => max_items,
            _ => max_items.saturating_mul(2),
        };

        let retriever = self.retrievers.get_or_build(&docs);
        // "No evidence" is an answer, not a tool failure, so it is Ok with an
        // explanation. An unresolvable document_id is an invalid argument and stays Err.
        match retriever.search(query, wanted, &HybridWeights::DEFAULT) {
            Ok(hits) => Ok(match mode {
                QueryMode::Evidence => {
                    ContextBuilder::build_evidence(query, &hits, budget).to_markdown()
                }
                QueryMode::Context => {
                    ContextBuilder::build_conceptual_context(query, &hits, &docs, budget)
                }
                QueryMode::Hits => {
                    serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".to_string())
                }
            }),
            Err(no_evidence) => Ok(no_evidence.to_markdown()),
        }
    }

    /// Retrieve the full content of a specific section with optional parent context and token budgeting.
    #[tool(
        name = "document_get_section",
        description = "Fetch the text and sub-clauses of a specific section by ID with token budgeting."
    )]
    pub async fn document_get_section(
        &self,
        params: Parameters<DocumentGetSectionParams>,
    ) -> Result<String, String> {
        let doc_id = &params.0.document_id;
        let section_id = &params.0.section_id;
        let include_parent = params.0.include_parent.unwrap_or(true);
        let budget = ContextBudget {
            max_tokens: params.0.max_tokens.unwrap_or(1500),
            max_chunks: 10,
            compact: true,
        };

        let doc = self.require_document(doc_id)?;
        match ContextBuilder::expand_section_context(&doc, section_id, include_parent, budget) {
            Some(content) => Ok(content),
            None => Err(format!(
                "Section '{section_id}' not found in document '{doc_id}'. \
                 Use 'document_outline' to list the section ids of this document."
            )),
        }
    }

    /// Read raw text from a specific page range with a character budget.
    #[tool(
        name = "document_read_pages",
        description = "Read sequential pages directly from a document with strict character bounds."
    )]
    pub async fn document_read_pages(
        &self,
        params: Parameters<DocumentReadPagesParams>,
    ) -> Result<String, String> {
        let doc_id = &params.0.document_id;
        let max_chars = params.0.max_chars.unwrap_or(8000).min(50000);

        if params.0.page_start == 0 {
            return Err("page_start must be >= 1 (PDF pages are 1-indexed)".to_string());
        }
        if params.0.page_start > params.0.page_end {
            return Err(format!(
                "page_start ({}) cannot be greater than page_end ({})",
                params.0.page_start, params.0.page_end
            ));
        }

        let Some(doc) = self.store.get(doc_id.trim()) else {
            return Err(self.unknown_document_error(doc_id));
        };

        if params.0.page_start > doc.metadata.total_pages {
            return Err(format!(
                "page_start ({}) exceeds total pages in document ({})",
                params.0.page_start, doc.metadata.total_pages
            ));
        }

        // Enforce maximum page span per call (tope de 30 páginas por llamada)
        let requested_end = params.0.page_end.min(doc.metadata.total_pages);
        let max_page_span = 30;
        let (page_end, range_capped) =
            if requested_end.saturating_sub(params.0.page_start) + 1 > max_page_span {
                (params.0.page_start + max_page_span - 1, true)
            } else {
                (requested_end, false)
            };

        let mut out = String::new();
        out.push_str(&format!(
            "# Lectura de Páginas: {} (pp. {}-{})\n\n",
            doc.metadata.title, params.0.page_start, page_end
        ));

        if range_capped {
            out.push_str(&format!(
                "> [!NOTE]\n> Rango limitado a {} páginas por llamada (solicitado hasta p. {}, ajustado a p. {}). Para leer más páginas, realice llamadas sucesivas.\n\n",
                max_page_span, params.0.page_end, page_end
            ));
        }

        let mut chars_count = 0;
        for p in params.0.page_start..=page_end {
            if let Some(page) = doc.get_page(p) {
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
        }
        Ok(out)
    }

    /// Render a specific document page to a high-resolution PNG image for visual multimodal inspection.
    #[tool(
        name = "document_render_page",
        description = "Render a specific document page to a high-resolution PNG image for visual inspection (diagrams, complex charts, scans) by Multimodal LLMs."
    )]
    pub async fn document_render_page(
        &self,
        params: Parameters<RenderPageParams>,
    ) -> Result<String, String> {
        let doc_id = &params.0.document_id;
        let page_num = params.0.page_number;
        let max_width = params.0.max_width.unwrap_or(1024);

        let doc = self.require_document(doc_id)?;
        match self
            .renderer
            .render_document_page(&doc, page_num, max_width)
        {
            Ok(rendered) => {
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
            Err(e) => Err(format!(
                "Failed to render page {page_num} of document '{doc_id}': {e}"
            )),
        }
    }

    /// Pull one structured artifact out of a document.
    #[tool(
        name = "document_extract",
        description = "Extract a structured artifact from a document: links (hyperlinks and cross-references with pages and coordinates), forms (AcroForm fields with names and values), or attachments (embedded files with MIME types and sizes)."
    )]
    pub async fn document_extract(
        &self,
        params: Parameters<DocumentExtractParams>,
    ) -> Result<String, String> {
        let p = params.0;
        match p.kind {
            ArtifactKind::Links => {
                self.document_get_links(Parameters(DocumentGetLinksParams {
                    document_id: p.document_id,
                    page: p.page,
                    kind: p.link_kind,
                }))
                .await
            }
            ArtifactKind::Forms => {
                self.document_get_forms(Parameters(DocumentGetFormsParams {
                    document_id: p.document_id,
                    page: p.page,
                    filled_only: p.filled_only,
                }))
                .await
            }
            ArtifactKind::Attachments => {
                self.document_get_attachments(Parameters(DocumentGetAttachmentsParams {
                    document_id: p.document_id,
                }))
                .await
            }
        }
    }

    /// Extract hyperlinks and internal cross-references from a document.
    pub async fn document_get_links(
        &self,
        params: Parameters<DocumentGetLinksParams>,
    ) -> Result<String, String> {
        let doc_id = &params.0.document_id;
        let page_filter = params.0.page;
        let kind_filter = params.0.kind.as_deref().unwrap_or("all").to_lowercase();

        let doc = self.require_document(doc_id)?;
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
    pub async fn document_get_forms(
        &self,
        params: Parameters<DocumentGetFormsParams>,
    ) -> Result<String, String> {
        let doc_id = &params.0.document_id;
        let page_filter = params.0.page;
        let filled_only = params.0.filled_only.unwrap_or(false);

        let doc = self.require_document(doc_id)?;
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
    pub async fn document_get_attachments(
        &self,
        params: Parameters<DocumentGetAttachmentsParams>,
    ) -> Result<String, String> {
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
    ) -> Result<String, String> {
        let doc_id = &params.0.document_id;
        let name_or_id = &params.0.name_or_id;
        let max_bytes = params.0.max_bytes.unwrap_or(524_288); // 512 KB default limit

        let doc = self.require_document(doc_id)?;
        if let Some(att) = doc.get_attachment(name_or_id) {
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
        } else {
            Err(format!(
                "Attachment '{name_or_id}' not found in document '{doc_id}'. \
                     Use 'document_get_attachments' to list the attachments of this document."
            ))
        }
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
