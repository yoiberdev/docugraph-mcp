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
    let list_json = server.document_list().await;

    // Verify valid JSON response structure
    let parsed: serde_json::Value = serde_json::from_str(&list_json).expect("valid JSON array");
    assert!(parsed.is_array());
    assert!(!parsed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_document_info_tool() {
    let server = DocuGraphServer::new();
    let info_json = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "test_doc_gof".to_string(),
        }))
        .await;

    let parsed: serde_json::Value = serde_json::from_str(&info_json).expect("valid JSON object");
    assert_eq!(parsed["id"], "test_doc_gof");
    assert!(parsed["sections_preview"].is_array());
}
