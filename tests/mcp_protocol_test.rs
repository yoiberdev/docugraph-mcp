use docugraph::document::model::{Document, DocumentMetadata};
use docugraph::mcp::{
    DocuGraphServer,
    tools::{DocumentInfoParams, PingParams},
};
use rmcp::{ServerHandler, handler::server::wrapper::Parameters};

#[tokio::test]
async fn test_server_info_and_capabilities() {
    let server = DocuGraphServer::new();
    let info = server.get_info();

    assert_eq!(info.server_info.name, "docugraph-mcp");
    assert!(info.capabilities.tools.is_some());
    assert!(info.instructions.is_some());
}

#[tokio::test]
async fn test_document_ping_tool() {
    let server = DocuGraphServer::new();

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
    let server = DocuGraphServer::new();
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
    let server = DocuGraphServer::new();
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
    });
    server.register_document(doc).await;

    let info_json = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "test_doc_gof".to_string(),
        }))
        .await;

    let parsed: serde_json::Value = serde_json::from_str(&info_json).expect("valid JSON object");
    assert_eq!(parsed["id"], "test_doc_gof");
    assert_eq!(parsed["title"], "GoF Design Patterns");
    assert!(parsed["sections_preview"].is_array());
}
