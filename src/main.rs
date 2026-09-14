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
        /// Path to the PDF file to ingest
        path: String,
    },
    /// List all currently indexed documents in cache
    List,
    /// Inspect the structure and outline of a document
    Info {
        /// Document identifier or filesystem path to PDF
        document: String,
    },
    /// Search across indexed documents using hybrid retrieval
    Search {
        /// Query keywords or natural language concept
        query: String,
        /// Maximum number of search results to display
        #[arg(short, long, default_value = "5")]
        limit: usize,
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
        Commands::Index { path } => {
            let p = std::path::Path::new(&path);
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
                    match docugraph::document::load_pdf_from_path(&pdf_path) {
                        Ok(doc) => {
                            let total_p = doc.metadata.total_pages;
                            let total_s = doc.total_sections();
                            if let Err(e) = store.insert(doc) {
                                eprintln!("failed to cache: {e}");
                            } else {
                                eprintln!("OK ({} pages, {} sections)", total_p, total_s);
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
                let doc = docugraph::document::load_pdf_from_path(&path)?;
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
                    eprintln!(
                        "  [{}] {} (ID: '{}', {} pages, {} sections)",
                        idx + 1,
                        doc.title,
                        doc.id,
                        doc.total_pages,
                        doc.total_sections
                    );
                }
                eprintln!();
            }
        }
        Commands::Info { document } => {
            info!(target: "cli", document = %document, "Retrieving document info");
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
                eprintln!("\n📄 Document: {}", doc.metadata.title);
                eprintln!("  ID:       {}", doc.id);
                eprintln!("  Pages:    {}", doc.metadata.total_pages);
                eprintln!("  Sections: {}", doc.total_sections());
                eprintln!("  Hash:     {}", doc.metadata.content_hash);
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
