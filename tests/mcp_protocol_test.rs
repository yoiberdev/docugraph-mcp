use docugraph::document::model::{Document, DocumentMetadata};
use docugraph::mcp::{
    DocuGraphServer,
    tools::{DocumentInfoParams, DocumentSearchParams, PingParams},
};
use docugraph::storage::DocumentStore;
use rmcp::{ServerHandler, handler::server::wrapper::Parameters};
use std::path::Path;
use tempfile::{TempDir, tempdir};

/// Server backed by a throwaway cache dir, so tests never write `.docugraph_cache` into the repo.
fn server_with_temp_cache() -> (DocuGraphServer, TempDir) {
    let cache = tempdir().expect("create temp cache dir");
    (DocuGraphServer::with_cache_dir(cache.path()), cache)
}

#[tokio::test]
async fn test_server_info_and_capabilities() {
    let (server, _cache) = server_with_temp_cache();
    let info = server.get_info();

    assert_eq!(info.server_info.name, "docugraph-mcp");
    assert!(info.capabilities.tools.is_some());
    assert!(info.instructions.is_some());
}

#[tokio::test]
async fn test_document_ping_tool() {
    let (server, _cache) = server_with_temp_cache();

    // Default ping message
    let resp = server
        .document_ping(Parameters(PingParams { message: None }))
        .await;
    assert!(resp.starts_with("pong: DocuGraph MCP is alive"));

    // Custom echo message
    let custom = server
        .document_ping(Parameters(PingParams {
            message: Some("hello agent".to_string()),
        }))
        .await;
    assert_eq!(custom, "pong: hello agent");
}

#[tokio::test]
async fn test_document_list_tool() {
    let (server, _cache) = server_with_temp_cache();
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
    let parsed: serde_json::Value = serde_json::from_str(&list_json).expect("valid JSON array");
    assert!(parsed.is_array());
    assert!(!parsed.as_array().unwrap().is_empty());
    assert!(
        parsed
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == "sample-doc")
    );
}

#[tokio::test]
async fn test_document_info_tool() {
    let (server, _cache) = server_with_temp_cache();
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
        .expect("tool call should succeed");

    let parsed: serde_json::Value = serde_json::from_str(&info_json).expect("valid JSON object");
    assert_eq!(parsed["id"], "test_doc_gof");
    assert_eq!(parsed["title"], "GoF Design Patterns");
    assert!(parsed["sections_preview"].is_array());
}

#[tokio::test]
async fn test_empty_cache_is_explained_with_its_absolute_path() {
    let (server, cache) = server_with_temp_cache();
    let cache_dir = std::path::absolute(cache.path()).expect("absolute cache path");

    let message = server.document_list().await;
    assert!(
        message.contains(&cache_dir.display().to_string()),
        "document_list should name the cache directory: {message}"
    );
    assert!(message.contains("DOCUGRAPH_CACHE_DIR"), "{message}");

    let err = server
        .document_search(Parameters(DocumentSearchParams {
            query: "anything".to_string(),
            document_id: None,
            limit: None,
        }))
        .await
        .expect_err("searching an empty cache is a tool error");
    assert!(err.message().contains("DOCUGRAPH_CACHE_DIR"), "{err}");
}

#[tokio::test]
async fn test_empty_in_memory_store_is_explained() {
    let server = DocuGraphServer::with_store(DocumentStore::new(None));
    let message = server.document_list().await;
    assert!(message.contains("no cache directory"), "{message}");
}

#[tokio::test]
async fn test_serve_startup_warnings() {
    let (server, cache) = server_with_temp_cache();

    let warnings = server.startup_warnings(cache.path());
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("no indexed documents"), "{warnings:?}");

    // Only the configured path is inspected here; nothing is created in the working directory.
    let warnings = server.startup_warnings(Path::new("relative-cache"));
    assert!(
        warnings.iter().any(|w| w.contains("is relative")),
        "{warnings:?}"
    );

    server
        .register_document(Document::new(DocumentMetadata {
            id: "indexed-doc".to_string(),
            title: "Indexed".to_string(),
            total_pages: 1,
            content_hash: "indexed-hash".to_string(),
            ..Default::default()
        }))
        .await;
    assert!(server.startup_warnings(cache.path()).is_empty());
}
