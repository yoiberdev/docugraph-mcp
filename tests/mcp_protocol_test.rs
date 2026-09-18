use docugraph::document::model::{Document, DocumentMetadata};
use docugraph::mcp::{DocuGraphServer, tools::DocumentInfoParams};
use docugraph::storage::DocumentStore;
use rmcp::{ServerHandler, handler::server::wrapper::Parameters};

/// A server backed by nothing but memory.
///
/// `DocuGraphServer::new()` binds the real cache directory, so `register_document`
/// used to persist these fixtures into it: they then showed up in `docugraph list`
/// as if they were the user's own documents, and in `docugraph bench`'s denominator.
fn in_memory_server() -> DocuGraphServer {
    DocuGraphServer::with_store(DocumentStore::new(None))
}

#[tokio::test]
async fn test_server_info_and_capabilities() {
    let server = in_memory_server();
    let info = server.get_info();

    assert_eq!(info.server_info.name, "docugraph-mcp");
    assert!(info.capabilities.tools.is_some());
    assert!(info.instructions.is_some());
}

/// The published tool surface, pinned, as a client over stdio actually sees it.
///
/// This server argues that an agent should spend few tokens, and every tool it
/// declares is spent in every session before a single question is asked: 15 tools
/// cost 2509 tokens of schema, which is the argument paying for itself in reverse.
/// Merging the four query tools into `document_query` and the three extractors
/// into `document_extract`, and dropping `document_ping`, brought that to 1907.
///
/// Pinning the list here is what stops it growing back one convenience at a time.
#[test]
fn test_tool_surface_is_the_published_one() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    const PUBLISHED: [&str; 9] = [
        "document_extract",
        "document_get_section",
        "document_info",
        "document_list",
        "document_outline",
        "document_query",
        "document_read_attachment",
        "document_read_pages",
        "document_render_page",
    ];

    let mut child = Command::new(env!("CARGO_BIN_EXE_docugraph"))
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the server binary must start");

    let stdin = child.stdin.as_mut().expect("stdin");
    for line in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ] {
        writeln!(stdin, "{line}").expect("write request");
    }
    drop(child.stdin.take());

    let out = child
        .wait_with_output()
        .expect("the server must exit on stdin close");
    let stdout = String::from_utf8_lossy(&out.stdout);

    let listing = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["id"] == 2)
        .expect("tools/list must be answered");

    let mut declared: Vec<String> = listing["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or_default().to_string())
        .collect();
    declared.sort();

    assert_eq!(
        declared, PUBLISHED,
        "the declared tools must be the ones the README documents; every extra one          is schema every session pays for before asking anything"
    );
}

#[tokio::test]
async fn test_document_list_tool() {
    let server = in_memory_server();
    let doc = Document::new(DocumentMetadata {
        id: "sample-doc".to_string(),
        title: "Sample Doc".to_string(),
        author: None,
        total_pages: 1,
        total_sections: 0,
        file_size_bytes: 100,
        content_hash: "hash123".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
        total_links: 0,
        ..Default::default()
    });
    server.register_document(doc).await;

    let list_json = server.document_list().await;
    let parsed: serde_json::Value = serde_json::from_str(&list_json).expect("valid JSON result");
    let documents = parsed["documents"].as_array().expect("documents array");
    assert!(!documents.is_empty());
    assert!(documents.iter().any(|d| d["id"] == "sample-doc"));
    assert!(
        parsed["cache_dir"].is_string(),
        "document_list must say which directory it read from"
    );
    assert!(
        parsed["hint"].is_null(),
        "a populated corpus needs no remediation hint"
    );
}

#[tokio::test]
async fn test_document_info_tool() {
    let server = in_memory_server();
    let doc = Document::new(DocumentMetadata {
        id: "test_doc_gof".to_string(),
        title: "GoF Design Patterns".to_string(),
        author: Some("GoF".to_string()),
        total_pages: 5,
        total_sections: 1,
        file_size_bytes: 500,
        content_hash: "hash_gof".to_string(),
        indexed_at: "2026-09-13T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
        total_links: 0,
        ..Default::default()
    });
    server.register_document(doc).await;

    let info_json = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "test_doc_gof".to_string(),
        }))
        .await
        .expect("info must succeed for an indexed document");

    let parsed: serde_json::Value = serde_json::from_str(&info_json).expect("valid JSON object");
    assert_eq!(parsed["id"], "test_doc_gof");
    assert_eq!(parsed["title"], "GoF Design Patterns");
    assert!(parsed["sections_preview"].is_array());
}

#[tokio::test]
async fn test_document_list_deterministic_order() {
    let server = in_memory_server();
    for id in ["zeta-doc", "alpha-doc", "mid-doc"] {
        server
            .register_document(Document::new(DocumentMetadata {
                id: id.to_string(),
                title: format!("Doc {}", id),
                total_pages: 1,
                content_hash: format!("hash-{}", id),
                indexed_at: "2026-09-14T00:00:00Z".to_string(),
                ..Default::default()
            }))
            .await;
    }

    let list_json = server.document_list().await;
    let parsed: serde_json::Value = serde_json::from_str(&list_json).expect("valid JSON result");
    let list = parsed["documents"].as_array().expect("documents array");
    let ids: Vec<&str> = list.iter().map(|d| d["id"].as_str().unwrap()).collect();

    // Verify list is strictly sorted alphabetically by id
    let mut sorted_ids = ids.clone();
    sorted_ids.sort();
    assert_eq!(
        ids, sorted_ids,
        "document_list must be strictly deterministic and sorted by id"
    );
}

#[tokio::test]
async fn test_read_pages_validation_and_span_limit() {
    use docugraph::mcp::tools::DocumentReadPagesParams;

    let server = in_memory_server();
    server
        .register_document(Document::new(DocumentMetadata {
            id: "big-book".to_string(),
            title: "Big Technical Book".to_string(),
            total_pages: 100,
            content_hash: "hash-big".to_string(),
            indexed_at: "2026-09-14T00:00:00Z".to_string(),
            ..Default::default()
        }))
        .await;

    // 1. page_start == 0 is rejected with Err (isError in MCP)
    let err_zero = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "big-book".to_string(),
            page_start: 0,
            page_end: 5,
            max_chars: None,
        }))
        .await;
    assert!(err_zero.is_err());
    assert!(err_zero.unwrap_err().contains("page_start must be >= 1"));

    // 2. page_start > page_end is rejected with Err
    let err_inverted = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "big-book".to_string(),
            page_start: 10,
            page_end: 5,
            max_chars: None,
        }))
        .await;
    assert!(err_inverted.is_err());
    assert!(
        err_inverted
            .unwrap_err()
            .contains("cannot be greater than page_end")
    );

    // 3. Document not found is rejected with Err
    let err_missing = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "non-existent".to_string(),
            page_start: 1,
            page_end: 5,
            max_chars: None,
        }))
        .await;
    assert!(err_missing.is_err());
    assert!(err_missing.unwrap_err().contains("not found"));

    // 4. Requesting > 30 pages applies the 30-page span cap
    let capped_read = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "big-book".to_string(),
            page_start: 1,
            page_end: 80,
            max_chars: None,
        }))
        .await
        .expect("read succeeds with range cap applied");

    assert!(
        capped_read.contains("pp. 1-30"),
        "Must cap end page to start + 29 (30 pages max)"
    );
    assert!(capped_read.contains("Rango limitado a 30 páginas"));
}

/// An empty corpus must say where it looked, not just return nothing.
///
/// Regression: `index` and `serve` are separate processes joined only by the cache
/// directory, so an empty corpus almost always means they resolved different ones.
/// A bare `[]` gave an agent no way to tell that from "nothing indexed yet", and
/// the resolved path appeared nowhere - not even under RUST_LOG=debug.
#[tokio::test]
async fn test_empty_corpus_reports_where_it_looked() {
    let server = in_memory_server();
    let json = server.document_list().await;
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON result");

    assert_eq!(parsed["total"], 0);
    assert!(parsed["documents"].as_array().unwrap().is_empty());

    let hint = parsed["hint"]
        .as_str()
        .expect("an empty corpus must carry a remediation hint");
    assert!(
        hint.contains("DOCUGRAPH_CACHE_DIR"),
        "the hint must name the variable that connects indexing to serving: {hint}"
    );
    assert!(
        hint.contains("docugraph index"),
        "the hint must name the command that fills the cache: {hint}"
    );
    assert!(
        parsed["cache_dir"].is_string(),
        "the directory searched must always be reported"
    );
}
