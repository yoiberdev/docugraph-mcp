use clap::{Parser, Subcommand};
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
    /// List all currently indexed documents
    List,
    /// Inspect the structure and outline of a document
    Info {
        /// Document identifier or filesystem path to PDF
        document: String,
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

    match cli.command {
        Commands::Serve => {
            info!("Starting DocuGraph MCP server on stdio transport...");
            eprintln!("DocuGraph MCP ready to accept JSON-RPC on stdin");
            docugraph::mcp::DocuGraphServer::serve_stdio().await?;
        }
        Commands::Index { path } => {
            info!(target: "cli", path = %path, "Indexing PDF document");
            let doc = docugraph::document::load_pdf_from_path(&path)?;
            eprintln!("\n📄 Document Ingested Successfully!");
            eprintln!("  ID:          {}", doc.id);
            eprintln!("  Title:       {}", doc.metadata.title);
            if let Some(author) = &doc.metadata.author {
                eprintln!("  Author:      {}", author);
            }
            eprintln!("  Pages:       {}", doc.metadata.total_pages);
            eprintln!("  Size:        {} bytes", doc.metadata.file_size_bytes);
            eprintln!("  SHA-256:     {}", doc.metadata.sha256_hash);
            eprintln!("  Sections:    {}", doc.total_sections());
            eprintln!("\n🌳 Document Outline:");
            print_outline_tree(&doc.sections, 0);
            eprintln!();
        }
        Commands::List => {
            info!(target: "cli", "Listing indexed documents");
            eprintln!(
                "To list active documents via MCP, use the 'document_list' tool in your agent client."
            );
            eprintln!("To index a document, run: docugraph index <file.pdf>");
        }
        Commands::Info { document } => {
            info!(target: "cli", document = %document, "Retrieving document info");
            let path = std::path::Path::new(&document);
            if path.exists() && path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                let doc = docugraph::document::load_pdf_from_path(path)?;
                eprintln!("\n📄 Document: {}", doc.metadata.title);
                eprintln!("  ID:       {}", doc.id);
                eprintln!("  Pages:    {}", doc.metadata.total_pages);
                eprintln!("  Sections: {}", doc.total_sections());
                eprintln!("\n🌳 Outline Preview:");
                print_outline_tree(&doc.sections, 0);
            } else {
                eprintln!("Document identifier or path: {}", document);
                eprintln!("To inspect a PDF directly, run: docugraph info path/to/file.pdf");
            }
        }
    }

    Ok(())
}

fn print_outline_tree(sections: &[docugraph::document::SectionNode], depth: usize) {
    for s in sections {
        let indent = "  ".repeat(depth);
        eprintln!(
            "{indent}├── {} [pp. {}-{}]",
            s.title, s.page_start, s.page_end
        );
        if !s.children.is_empty() {
            print_outline_tree(&s.children, depth + 1);
        }
    }
}
