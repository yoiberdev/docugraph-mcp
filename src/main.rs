use clap::{Parser, Subcommand};
use docugraph::retrieval::HybridRetriever;
use docugraph::storage::{DiskCache, DocumentStore};
use rmcp::ServiceExt;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

#[derive(Parser, Debug)]
#[command(
    name = "docugraph",
    version,
    about = "Universal Document Knowledge Graph MCP Server for AI Agents",
    long_about = "DocuGraph indexes technical digital PDFs into structured knowledge graphs, \
                  enabling AI agents to navigate and retrieve compact, evidence-backed context."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the Model Context Protocol (MCP) server over stdio
    Serve,
    /// Index a PDF document into the knowledge graph
    Index {
        /// Path to the PDF file or directory to ingest
        path: String,
        /// Optional password for encrypted or password-protected PDFs
        #[arg(short, long)]
        password: Option<String>,
    },
    /// List all currently indexed documents in cache
    List,
    /// Inspect the structure and outline of a document
    Info {
        /// Document identifier or filesystem path to PDF
        document: String,
        /// Optional password for encrypted PDF if loading from file path
        #[arg(short, long)]
        password: Option<String>,
    },
    /// Search across indexed documents using hybrid retrieval
    Search {
        /// Query keywords or natural language concept
        query: String,
        /// Maximum number of search results to display
        #[arg(short, long, default_value = "5")]
        limit: usize,
    },
    /// Render a page of a document to a PNG image file
    Render {
        /// Document identifier or filesystem path to PDF
        document: String,
        /// Page number to render (1-based)
        #[arg(short, long, default_value = "1")]
        page: u32,
        /// Output PNG file path (default: <doc_id>_p<page>.png)
        #[arg(short, long)]
        out: Option<String>,
        /// Maximum resolution width in pixels (default: 1024)
        #[arg(short, long, default_value = "1024")]
        width: u32,
    },
    /// Run precision, recall, and token reduction benchmark on evaluation dataset
    Bench {
        /// Path to evaluation questions JSON file (default: evaluation/questions.json)
        #[arg(short, long, default_value = "evaluation/questions.json")]
        eval: String,
        /// Budget strategy: aggressive, balanced, or exhaustive (default: balanced)
        #[arg(short, long, default_value = "balanced")]
        strategy: String,
        /// Optional target document ID or file path to evaluate against
        #[arg(short, long)]
        document: Option<String>,
        /// Optional path to save markdown summary report
        #[arg(short, long)]
        out: Option<String>,
    },
    /// Extract hyperlinks and internal cross-references
    Links {
        /// Document identifier or filesystem path to PDF
        document: String,
        /// Optional page number filter (1-based)
        #[arg(short, long)]
        page: Option<u32>,
        /// Filter link kind: all, external, internal
        #[arg(short, long, default_value = "all")]
        kind: String,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Extract interactive form fields (AcroForms) and their values
    Forms {
        /// Document identifier or filesystem path to PDF
        document: String,
        /// Optional page number filter (1-based)
        #[arg(short, long)]
        page: Option<u32>,
        /// Filter to only fields that contain a non-empty value
        #[arg(long)]
        filled_only: bool,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Extract and inspect embedded file attachments (e.g. ZUGFeRD XML, CSV, datasets)
    Attachments {
        /// Document identifier or filesystem path to PDF
        document: String,
        /// Optional specific attachment filename or ID to inspect or extract
        #[arg(short, long)]
        name: Option<String>,
        /// Optional directory path to extract and save the attachment(s)
        #[arg(short, long)]
        extract_dir: Option<String>,
        /// Output format: text or json
        #[arg(long, default_value = "text")]
        format: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // CRITICAL: Log strictly to stderr so stdio stdout remains 100% clean for JSON-RPC MCP frames
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default tracing subscriber failed");

    let cli = Cli::parse();
    let cache = DiskCache::new(DiskCache::default_dir()).ok();
    let store = DocumentStore::new(cache);

    match cli.command {
        Commands::Serve => {
            info!("Starting DocuGraph MCP server on stdio transport...");
            eprintln!("DocuGraph MCP ready to accept JSON-RPC on stdin");
            let server = docugraph::mcp::DocuGraphServer::with_store(store);
            let service = server.serve(rmcp::transport::stdio()).await?;
            service.waiting().await?;
        }
        Commands::Index { path, password } => {
            let p = std::path::Path::new(&path);
            let pwd = password.as_deref();
            if p.is_dir() {
                info!(target: "cli", dir = %path, "Indexing all PDF documents in directory");
                let mut pdf_paths = Vec::new();
                collect_pdfs_recursive(p, &mut pdf_paths);

                if pdf_paths.is_empty() {
                    eprintln!("No PDF files found in directory: {}", path);
                    return Ok(());
                }

                eprintln!(
                    "\n📚 Found {} PDF document(s) in '{}'. Indexing...",
                    pdf_paths.len(),
                    path
                );
                let mut indexed_count = 0;
                let total_found = pdf_paths.len();
                for pdf_path in pdf_paths {
                    eprint!("  - Indexing {}... ", pdf_path.display());
                    match docugraph::document::load_pdf_from_path_with_password(&pdf_path, pwd) {
                        Ok(doc) => {
                            let total_p = doc.metadata.total_pages;
                            let total_s = doc.total_sections();
                            let sec_flag = if doc.metadata.untrusted_text_detected {
                                " [⚠️ UNTRUSTED TEXT]"
                            } else {
                                ""
                            };
                            let scan_flag = if doc.metadata.scanned_pages_count > 0 {
                                format!(" [📷 SCANNED: {}p]", doc.metadata.scanned_pages_count)
                            } else {
                                String::new()
                            };
                            if let Err(e) = store.insert(doc) {
                                eprintln!("failed to cache: {e}");
                            } else {
                                eprintln!(
                                    "OK ({} pages, {} sections{}{})",
                                    total_p, total_s, sec_flag, scan_flag
                                );
                                indexed_count += 1;
                            }
                        }
                        Err(err) => {
                            eprintln!("failed: {err}");
                        }
                    }
                }
                eprintln!(
                    "\n✅ Successfully indexed and cached {}/{} document(s)!",
                    indexed_count, total_found
                );
            } else {
                info!(target: "cli", path = %path, "Indexing PDF document");
                let doc = docugraph::document::load_pdf_from_path_with_password(&path, pwd)?;
                store.insert(doc.clone())?;

                eprintln!("\n📄 Document Ingested Successfully!");
                eprintln!("  ID:          {}", doc.id);
                eprintln!("  Title:       {}", doc.metadata.title);
                if let Some(author) = &doc.metadata.author {
                    eprintln!("  Author:      {}", author);
                }
                eprintln!("  Pages:       {}", doc.metadata.total_pages);
                eprintln!("  Size:        {} bytes", doc.metadata.file_size_bytes);
                eprintln!("  SHA-256:     {}", doc.metadata.content_hash);
                eprintln!("  Sections:    {}", doc.total_sections());
                if doc.metadata.is_encrypted {
                    eprintln!("  Encrypted:   Yes (Successfully Decrypted)");
                }
                if doc.metadata.untrusted_text_detected {
                    eprintln!("  Security:    ⚠️ Untrusted hidden or microscopic text detected!");
                } else {
                    eprintln!("  Security:    Clean (No hidden/microscopic text)");
                }
                if doc.metadata.scanned_pages_count > 0 {
                    eprintln!(
                        "  📷 Escaneado:   ⚠️ {} página(s) sin capa de texto digital (OCR recomendado)",
                        doc.metadata.scanned_pages_count
                    );
                } else {
                    eprintln!("  📷 Escaneado:   No (Documento digital)");
                }
                if doc.metadata.has_forms {
                    eprintln!(
                        "  📋 Formularios: {} campo(s) interactivo(s) AcroForm",
                        doc.metadata.total_form_fields
                    );
                }
                if doc.metadata.is_tagged {
                    eprintln!("  🏷️ Tagged PDF:  Sí (Estructura semántica /StructTreeRoot)");
                }
                if doc.metadata.has_attachments {
                    eprintln!(
                        "  📎 Adjuntos:    {} archivo(s) incrustado(s)",
                        doc.metadata.total_attachments
                    );
                }
                eprintln!("\n🌳 Document Outline:");
                print_outline_tree(&doc.sections, 0);
                eprintln!();
            }
        }
        Commands::List => {
            info!(target: "cli", "Listing indexed documents");
            let docs = store.list_documents();
            if docs.is_empty() {
                eprintln!("No documents cached yet. Index a PDF using: docugraph index <file.pdf>");
            } else {
                eprintln!("\n📚 Indexed Documents in Cache ({} total):", docs.len());
                for (idx, doc) in docs.iter().enumerate() {
                    let sec_status = if doc.untrusted_text_detected {
                        " [⚠️ UNTRUSTED]"
                    } else {
                        ""
                    };
                    eprintln!(
                        "  [{}] {} (ID: '{}', {} pages, {} sections{})",
                        idx + 1,
                        doc.title,
                        doc.id,
                        doc.total_pages,
                        doc.total_sections,
                        sec_status
                    );
                }
                eprintln!();
            }
        }
        Commands::Info { document, password } => {
            info!(target: "cli", document = %document, "Retrieving document info");
            let pwd = password.as_deref();
            let doc = if let Some(d) = store.get(&document) {
                Some(d)
            } else {
                let path = std::path::Path::new(&document);
                if path.exists() && path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                    Some(docugraph::document::load_pdf_from_path_with_password(
                        path, pwd,
                    )?)
                } else {
                    None
                }
            };

            if let Some(doc) = doc {
                eprintln!("\n📄 Document: {}", doc.metadata.title);
                eprintln!("  ID:        {}", doc.id);
                eprintln!("  Pages:     {}", doc.metadata.total_pages);
                eprintln!("  Sections:  {}", doc.total_sections());
                eprintln!("  Hash:      {}", doc.metadata.content_hash);
                eprintln!(
                    "  Encrypted: {}",
                    if doc.metadata.is_encrypted {
                        "Yes"
                    } else {
                        "No"
                    }
                );
                eprintln!(
                    "  Security:  {}",
                    if doc.metadata.untrusted_text_detected {
                        "⚠️ Untrusted hidden or microscopic text detected!"
                    } else {
                        "Clean"
                    }
                );
                eprintln!(
                    "  Escaneado: {}",
                    if doc.metadata.scanned_pages_count > 0 {
                        format!(
                            "⚠️ {} página(s) sin capa de texto digital (OCR recomendado)",
                            doc.metadata.scanned_pages_count
                        )
                    } else {
                        "No (Documento digital)".to_string()
                    }
                );
                eprintln!("  Links:     {}", doc.metadata.total_links);
                eprintln!("  Forms:     {} field(s)", doc.metadata.total_form_fields);
                eprintln!(
                    "  Tagged:    {}",
                    if doc.metadata.is_tagged {
                        "Yes (Semantic StructTreeRoot / PDF/UA)"
                    } else {
                        "No"
                    }
                );
                eprintln!(
                    "  Adjuntos:  {} archivo(s) incrustado(s)",
                    doc.metadata.total_attachments
                );
                eprintln!("\n🌳 Outline Preview:");
                print_outline_tree(&doc.sections, 0);
            } else {
                eprintln!("Document not found: {}", document);
                eprintln!("Try: docugraph index <path.pdf> or docugraph list");
            }
        }
        Commands::Search { query, limit } => {
            info!(target: "cli", query = %query, "Performing hybrid search");
            let metas = store.list_documents();
            let docs: Vec<_> = metas.into_iter().filter_map(|m| store.get(&m.id)).collect();

            if docs.is_empty() {
                eprintln!("No indexed documents available to search. Index a PDF first.");
                return Ok(());
            }

            let retriever = HybridRetriever::build(&docs, None, None);
            let hits = retriever.search(&query, limit);

            if hits.is_empty() {
                eprintln!("No matches found for query: '{}'", query);
            } else {
                eprintln!("\n🔍 Search Results for '{}':", query);
                for (idx, hit) in hits.iter().enumerate() {
                    eprintln!(
                        "\n  [{}] {} (Score: {:.3} | pp. {}-{})",
                        idx + 1,
                        hit.title,
                        hit.final_score,
                        hit.page_start,
                        hit.page_end
                    );
                    eprintln!("      {}", hit.snippet);
                }
                eprintln!();
            }
        }
        Commands::Render {
            document,
            page,
            out,
            width,
        } => {
            info!(target: "cli", document = %document, page = page, "Rendering page to PNG");
            let cache_dir = DiskCache::default_dir();
            let renderer = docugraph::multimodal::CachedPageRendererProxy::new(Some(&cache_dir));

            let doc = if let Some(d) = store.get(&document) {
                Some(d)
            } else {
                let path = std::path::Path::new(&document);
                if path.exists() && path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                    Some(docugraph::document::load_pdf_from_path(path)?)
                } else {
                    None
                }
            };

            if let Some(doc) = doc {
                let rendered = renderer.render_document_page(&doc, page, width)?;
                let output_path = out.unwrap_or_else(|| format!("{}_p{}.png", doc.id, page));
                std::fs::write(&output_path, &rendered.png_bytes)?;
                eprintln!("\n🖼️ Page Rendered Successfully!");
                eprintln!("  Document:    {}", doc.metadata.title);
                eprintln!("  Page:        {}", page);
                eprintln!("  Dimensions:  {}x{} px", rendered.width, rendered.height);
                eprintln!("  Output:      {}", output_path);
                eprintln!(
                    "  Cached:      {}",
                    if rendered.from_cache {
                        "Yes (Disk Hit)"
                    } else {
                        "No (Fresh Rasterization)"
                    }
                );
            } else {
                eprintln!("Document not found: {}", document);
            }
        }
        Commands::Bench {
            eval,
            strategy,
            document,
            out,
        } => {
            info!(target: "cli", eval = %eval, strategy = %strategy, "Running context benchmark");
            let questions = docugraph::benchmark::BenchmarkRunner::load_questions_from_file(&eval)?;

            let docs = if let Some(ref doc_id) = document {
                if let Some(d) = store.get(doc_id) {
                    vec![d]
                } else {
                    let path = std::path::Path::new(doc_id);
                    if path.exists() && path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                        vec![docugraph::document::load_pdf_from_path(path)?]
                    } else {
                        eprintln!(
                            "Document '{}' not found. Evaluating on benchmark reference document.",
                            doc_id
                        );
                        vec![docugraph::benchmark::create_benchmark_sample_document()]
                    }
                }
            } else {
                let metas = store.list_documents();
                let mut d_list: Vec<_> =
                    metas.into_iter().filter_map(|m| store.get(&m.id)).collect();
                d_list.push(docugraph::benchmark::create_benchmark_sample_document());
                d_list
            };

            let budget_strategy: Box<dyn docugraph::benchmark::BudgetStrategy> =
                match strategy.to_lowercase().as_str() {
                    "aggressive" => Box::new(docugraph::benchmark::AggressiveBudgetStrategy),
                    "exhaustive" => Box::new(docugraph::benchmark::ExhaustiveBudgetStrategy),
                    _ => Box::new(docugraph::benchmark::BalancedBudgetStrategy),
                };

            let mut runner = docugraph::benchmark::BenchmarkRunner::new(budget_strategy);
            runner.add_observer(std::sync::Arc::new(
                docugraph::benchmark::ConsoleBenchmarkObserver,
            ));

            let report = runner.run_suite(&questions, &docs)?;

            if let Some(out_path) = out {
                std::fs::write(&out_path, report.to_markdown_summary())?;
                eprintln!("📝 Benchmark markdown report saved to: {}", out_path);
            }
        }
        Commands::Links {
            document,
            page,
            kind,
            format,
        } => {
            info!(target: "cli", document = %document, "Extracting document links");
            let doc = if let Some(d) = store.get(&document) {
                Some(d)
            } else {
                let path = std::path::Path::new(&document);
                if path.exists() && path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                    Some(docugraph::document::load_pdf_from_path(path)?)
                } else {
                    None
                }
            };

            if let Some(doc) = doc {
                let kind_filter = kind.to_lowercase();
                let pages: Vec<&docugraph::document::Page> = if let Some(p) = page {
                    doc.get_page(p).into_iter().collect()
                } else {
                    doc.pages.iter().collect()
                };

                let mut matched_links = Vec::new();
                for p in pages {
                    for link in &p.links {
                        let matches = match kind_filter.as_str() {
                            "external" => link.is_external(),
                            "internal" => link.is_internal(),
                            _ => true,
                        };
                        if matches {
                            matched_links.push(link);
                        }
                    }
                }

                if format.eq_ignore_ascii_case("json") {
                    let results: Vec<docugraph::mcp::DocumentLinkResult> = matched_links
                        .iter()
                        .map(|link| docugraph::mcp::DocumentLinkResult {
                            page_number: link.page_number,
                            kind: match &link.target {
                                docugraph::document::LinkTarget::Uri(_) => "external".to_string(),
                                docugraph::document::LinkTarget::InternalPage(_) => {
                                    "internal".to_string()
                                }
                                docugraph::document::LinkTarget::Named(_) => "named".to_string(),
                            },
                            uri: link.uri.clone(),
                            target_page: link.target_page,
                            named_target: match &link.target {
                                docugraph::document::LinkTarget::Named(n) => Some(n.clone()),
                                _ => None,
                            },
                            rect: link.rect,
                        })
                        .collect();

                    let res = docugraph::mcp::DocumentGetLinksResult {
                        document_id: doc.id.0.clone(),
                        total_links: results.len(),
                        links: results,
                    };
                    println!("{}", serde_json::to_string_pretty(&res)?);
                } else {
                    eprintln!("\n🔗 Extracted Hyperlinks for '{}':", doc.metadata.title);
                    eprintln!("  Total Links: {}", matched_links.len());
                    if matched_links.is_empty() {
                        eprintln!("  No links found matching filter criteria.");
                    } else {
                        eprintln!();
                        for (idx, link) in matched_links.iter().enumerate() {
                            let (target_desc, kind_desc) = match &link.target {
                                docugraph::document::LinkTarget::Uri(u) => {
                                    (u.clone(), "External URI")
                                }
                                docugraph::document::LinkTarget::InternalPage(p) => {
                                    (format!("Page {}", p), "Internal GoTo")
                                }
                                docugraph::document::LinkTarget::Named(n) => {
                                    (format!("Named Dest: {}", n), "Named Destination")
                                }
                            };
                            let rect_desc = match link.rect {
                                Some([x0, y0, x1, y1]) => {
                                    format!(" [rect: {:.1}, {:.1}, {:.1}, {:.1}]", x0, y0, x1, y1)
                                }
                                None => String::new(),
                            };
                            eprintln!(
                                "  [{}] p.{} | {:<18} | {}{}",
                                idx + 1,
                                link.page_number,
                                kind_desc,
                                target_desc,
                                rect_desc
                            );
                        }
                        eprintln!();
                    }
                }
            } else {
                eprintln!("Document not found: {}", document);
            }
        }
        Commands::Forms {
            document,
            page,
            filled_only,
            format,
        } => {
            info!(target: "cli", document = %document, "Extracting interactive form fields");
            let doc = if let Some(d) = store.get(&document) {
                Some(d)
            } else {
                let path = std::path::Path::new(&document);
                if path.exists() && path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                    Some(docugraph::document::load_pdf_from_path(path)?)
                } else {
                    None
                }
            };

            if let Some(doc) = doc {
                let mut matched_fields: Vec<&docugraph::document::FormField> = if let Some(p) = page
                {
                    doc.forms_for_page(p)
                } else {
                    doc.forms.iter().collect()
                };

                if filled_only {
                    matched_fields
                        .retain(|f| f.value.as_ref().is_some_and(|v| !v.trim().is_empty()));
                }

                if format.eq_ignore_ascii_case("json") {
                    let results: Vec<docugraph::mcp::FormFieldResult> = matched_fields
                        .iter()
                        .map(|f| docugraph::mcp::FormFieldResult {
                            name: f.name.clone(),
                            fully_qualified_name: f.fully_qualified_name.clone(),
                            field_type: f.field_type.as_str().to_string(),
                            value: f.value.clone(),
                            default_value: f.default_value.clone(),
                            read_only: f.read_only,
                            required: f.required,
                            page_number: f.page_number,
                            rect: f.rect,
                        })
                        .collect();

                    let res = docugraph::mcp::DocumentGetFormsResult {
                        document_id: doc.id.0.clone(),
                        total_fields: results.len(),
                        fields: results,
                    };
                    println!("{}", serde_json::to_string_pretty(&res)?);
                } else {
                    eprintln!("\n📋 Interactive Form Fields for '{}':", doc.metadata.title);
                    eprintln!("  Total Fields: {}", matched_fields.len());
                    if matched_fields.is_empty() {
                        eprintln!("  No form fields found matching filter criteria.");
                    } else {
                        eprintln!();
                        for (idx, field) in matched_fields.iter().enumerate() {
                            let val_desc = field
                                .value
                                .as_deref()
                                .map(|v| format!("= \"{}\"", v))
                                .unwrap_or_else(|| "(empty)".to_string());
                            let page_desc = field
                                .page_number
                                .map(|p| format!("p.{}", p))
                                .unwrap_or_else(|| "p.?".to_string());
                            let flags_desc = match (field.read_only, field.required) {
                                (true, true) => " [RO, REQ]",
                                (true, false) => " [RO]",
                                (false, true) => " [REQ]",
                                (false, false) => "",
                            };
                            let rect_desc = match field.rect {
                                Some([x0, y0, x1, y1]) => {
                                    format!(" [rect: {:.1}, {:.1}, {:.1}, {:.1}]", x0, y0, x1, y1)
                                }
                                None => String::new(),
                            };
                            eprintln!(
                                "  [{}] {:<5} | {:<10} | {:<25} | {}{}{}",
                                idx + 1,
                                page_desc,
                                field.field_type.as_str(),
                                field.fully_qualified_name,
                                val_desc,
                                flags_desc,
                                rect_desc
                            );
                        }
                        eprintln!();
                    }
                }
            } else {
                eprintln!("Document not found: {}", document);
            }
        }
        Commands::Attachments {
            document,
            name,
            extract_dir,
            format,
        } => {
            info!(target: "cli", document = %document, "Inspecting embedded file attachments");
            let doc = if let Some(d) = store.get(&document) {
                Some(d)
            } else {
                let path = std::path::Path::new(&document);
                if path.exists() && path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                    Some(docugraph::document::load_pdf_from_path(path)?)
                } else {
                    None
                }
            };

            if let Some(doc) = doc {
                if let Some(target_name) = name {
                    if let Some(att) = doc.get_attachment(&target_name) {
                        if let Some(ref dir) = extract_dir {
                            std::fs::create_dir_all(dir)?;
                            let out_path = std::path::Path::new(dir).join(&att.filename);
                            std::fs::write(&out_path, &att.data)?;
                            eprintln!(
                                "💾 Extracted attachment '{}' to: {}",
                                att.filename,
                                out_path.display()
                            );
                        } else if format.eq_ignore_ascii_case("json") {
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&att.data);
                            let res = docugraph::mcp::DocumentReadAttachmentResult {
                                document_id: doc.id.0.clone(),
                                filename: att.filename.clone(),
                                mime_type: att.mime_type.clone(),
                                size_bytes: att.size_bytes,
                                encoding: if att.is_text {
                                    "text".to_string()
                                } else {
                                    "base64".to_string()
                                },
                                content: if att.is_text {
                                    String::from_utf8_lossy(&att.data).to_string()
                                } else {
                                    b64
                                },
                                truncated: false,
                            };
                            println!("{}", serde_json::to_string_pretty(&res)?);
                        } else if att.is_text {
                            println!("{}", String::from_utf8_lossy(&att.data));
                        } else {
                            eprintln!(
                                "Binary attachment: {} ({} bytes, mime: {:?})",
                                att.filename, att.size_bytes, att.mime_type
                            );
                            eprintln!(
                                "To extract, use: docugraph attachments \"{}\" --name \"{}\" --extract-dir <dir>",
                                document, att.filename
                            );
                        }
                    } else {
                        eprintln!(
                            "Attachment '{}' not found in document '{}'.",
                            target_name, document
                        );
                    }
                } else if let Some(ref dir) = extract_dir {
                    std::fs::create_dir_all(dir)?;
                    for att in &doc.attachments {
                        let out_path = std::path::Path::new(dir).join(&att.filename);
                        std::fs::write(&out_path, &att.data)?;
                        eprintln!("💾 Extracted '{}' ({} bytes)", att.filename, att.size_bytes);
                    }
                    eprintln!(
                        "✅ Extracted {} attachment(s) to: {}",
                        doc.attachments.len(),
                        dir
                    );
                } else if format.eq_ignore_ascii_case("json") {
                    let summaries: Vec<docugraph::mcp::AttachmentSummaryResult> = doc
                        .attachments
                        .iter()
                        .map(|att| docugraph::mcp::AttachmentSummaryResult {
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

                    let res = docugraph::mcp::DocumentGetAttachmentsResult {
                        document_id: doc.id.0.clone(),
                        total_attachments: summaries.len(),
                        attachments: summaries,
                    };
                    println!("{}", serde_json::to_string_pretty(&res)?);
                } else {
                    eprintln!(
                        "\n📎 Embedded File Attachments for '{}':",
                        doc.metadata.title
                    );
                    eprintln!("  Total Attachments: {}", doc.attachments.len());
                    if doc.attachments.is_empty() {
                        eprintln!("  No embedded file attachments found.");
                    } else {
                        eprintln!();
                        for (idx, att) in doc.attachments.iter().enumerate() {
                            let mime = att
                                .mime_type
                                .as_deref()
                                .unwrap_or("application/octet-stream");
                            let kind = if att.is_text { "Text" } else { "Binary" };
                            let desc = att
                                .description
                                .as_deref()
                                .map(|d| format!(" ({})", d))
                                .unwrap_or_default();
                            eprintln!(
                                "  [{}] {:<30} | {:>8} bytes | {:<6} | {}{}",
                                idx + 1,
                                att.filename,
                                att.size_bytes,
                                kind,
                                mime,
                                desc
                            );
                        }
                        eprintln!();
                    }
                }
            } else {
                eprintln!("Document not found: {}", document);
            }
        }
    }

    Ok(())
}

fn print_outline_tree(sections: &[docugraph::document::SectionNode], depth: usize) {
    let indent = "  ".repeat(depth);
    for s in sections {
        eprintln!(
            "{}* {} (pp. {}-{}, ID: `{}`)",
            indent, s.title, s.page_start, s.page_end, s.id
        );
        if depth < 3 {
            print_outline_tree(&s.children, depth + 1);
        }
    }
}

fn collect_pdfs_recursive(dir: &std::path::Path, acc: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_pdfs_recursive(&path, acc);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("pdf"))
                .unwrap_or(false)
            {
                acc.push(path);
            }
        }
    }
}
