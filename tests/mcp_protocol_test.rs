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
    let server = DocuGraphServer::new();
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
    let list: Vec<serde_json::Value> = serde_json::from_str(&list_json).expect("valid JSON array");
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

    let server = DocuGraphServer::new();
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
