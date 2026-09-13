use clap::{Parser, Subcommand};
use tracing::{info, Level};
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
    /// Index a PDF document or directory into the knowledge graph
    Index {
        /// Path to the PDF file or folder containing PDFs
        path: String,
    },
    /// List all currently indexed documents
    List,
    /// Inspect the structure and metadata of an indexed document
    Info {
        /// Document identifier or filename
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
            docugraph::mcp::server::run_stdio_server().await?;
        }
        Commands::Index { path } => {
            info!(target: "cli", path = %path, "Indexing document");
            eprintln!("Indexing requested for: {}", path);
        }
        Commands::List => {
            info!(target: "cli", "Listing indexed documents");
            eprintln!("No documents indexed yet. Run 'docugraph index <file.pdf>'");
        }
        Commands::Info { document } => {
            info!(target: "cli", document = %document, "Retrieving document info");
            eprintln!("Information for document: {}", document);
        }
    }

    Ok(())
}
