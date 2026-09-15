use docugraph::document::model::{Document, DocumentMetadata, Page, SectionNode};
use docugraph::mcp::{DocuGraphServer, tools::*};
use docugraph::storage::DocumentStore;
use rmcp::handler::server::wrapper::Parameters;

fn create_test_server() -> DocuGraphServer {
    let store = DocumentStore::new(None); // Pure in-memory for testing

    let mut doc = Document::new(DocumentMetadata {
        id: "git-guide".to_string(),
        title: "Aprendiendo Git".to_string(),
        author: Some("Test Author".to_string()),
        total_pages: 5,
        total_sections: 3,
        file_size_bytes: 2048,
        content_hash: "hashgit123".to_string(),
        indexed_at: "2026-09-13T12:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
        total_links: 0,
        ..Default::default()
    });

    doc.add_page(Page::new(
        1,
        "Capítulo 1: Introducción a Git y Control de Versiones.",
    ));
    doc.add_page(Page::new(
        2,
        "1.1 Ramas Locales y Comandos Básicos.\nEl comando git branch permite crear, listar y eliminar ramas locales de trabajo.",
    ));
    doc.add_page(Page::new(
        3,
        "1.2 Fusión y Resolución de Conflictos.\nCuando dos ramas modifican las mismas líneas de un archivo ocurre un conflicto.",
    ));

    let mut ch1 = SectionNode::new("cap-1", "Capítulo 1: Introducción", 1, 1, 3, None);
    let sec1 = SectionNode::new(
        "ramas-locales",
        "1.1 Ramas Locales",
        2,
        2,
        2,
        Some("cap-1".to_string()),
    );
    let sec2 = SectionNode::new(
        "fusion-conflictos",
        "1.2 Fusión y Conflictos",
        2,
        3,
        3,
        Some("cap-1".to_string()),
    );
    ch1.add_child(sec1);
    ch1.add_child(sec2);
    doc.sections.push(ch1);

    store.insert(doc).expect("insert must succeed");
    DocuGraphServer::with_store(store)
}

#[tokio::test]
async fn test_mcp_document_outline() {
    let server = create_test_server();
    let outline_json = server
        .document_outline(Parameters(DocumentOutlineParams {
            document_id: "git-guide".to_string(),
            max_depth: Some(2),
        }))
        .await;

    let tree: serde_json::Value = serde_json::from_str(&outline_json).expect("valid JSON tree");
    assert!(tree.is_array());
    let root = &tree[0];
    assert_eq!(root["title"], "Capítulo 1: Introducción");
    assert_eq!(root["children"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_mcp_document_search_bm25() {
    let server = create_test_server();
    let resp = server
        .document_search(Parameters(DocumentSearchParams {
            query: "ramas locales trabajo".to_string(),
            document_id: Some("git-guide".to_string()),
            limit: Some(3),
        }))
        .await;

    let hits: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON hits");
    assert!(hits.is_array());
    assert!(!hits.as_array().unwrap().is_empty());
    assert_eq!(hits[0]["document_id"], "git-guide");
}

#[tokio::test]
async fn test_mcp_document_search_hybrid() {
    let server = create_test_server();
    let resp = server
        .document_search_hybrid(Parameters(DocumentSearchHybridParams {
            query: "fusión y resolución de conflictos".to_string(),
            document_id: None,
            limit: Some(2),
            bm25_weight: Some(0.6),
            semantic_weight: Some(0.2),
            structural_weight: Some(0.2),
        }))
        .await;

    let hits: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON hybrid hits");
    assert!(hits.is_array());
    assert!(!hits.as_array().unwrap().is_empty());
    assert!(hits[0]["final_score"].as_f64().unwrap() > 0.0);
}

#[tokio::test]
async fn test_mcp_document_get_section() {
    let server = create_test_server();
    let content = server
        .document_get_section(Parameters(DocumentGetSectionParams {
            document_id: "git-guide".to_string(),
            section_id: "ramas-locales".to_string(),
            include_parent: Some(true),
            max_tokens: Some(500),
        }))
        .await
        .expect("get_section must succeed for existing section");

    assert!(content.contains("1.1 Ramas Locales"));
    assert!(content.contains("Sección Padre"));
    assert!(content.contains("git branch"));
}

#[tokio::test]
async fn test_mcp_document_get_evidence() {
    let server = create_test_server();
    let evidence_md = server
        .document_get_evidence(Parameters(DocumentGetEvidenceParams {
            query: "ramas locales".to_string(),
            document_id: None,
            max_tokens: Some(500),
            max_items: Some(2),
        }))
        .await;

    assert!(evidence_md.contains("Evidencia Recuperada"));
    assert!(evidence_md.contains("[Doc: git-guide"));
}

#[tokio::test]
async fn test_mcp_document_read_pages() {
    let server = create_test_server();
    let pages_md = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "git-guide".to_string(),
            page_start: 2,
            page_end: 3,
            max_chars: Some(1000),
        }))
        .await
        .expect("read_pages must succeed for valid range");

    assert!(pages_md.contains("Página 2"));
    assert!(pages_md.contains("Página 3"));
    assert!(pages_md.contains("git branch"));
}

#[tokio::test]
async fn test_mcp_document_get_context() {
    let server = create_test_server();
    let context_md = server
        .document_get_context(Parameters(DocumentGetContextParams {
            query: "ramas locales de trabajo".to_string(),
            document_id: Some("git-guide".to_string()),
            max_tokens: Some(600),
            max_chunks: Some(2),
        }))
        .await;

    assert!(context_md.contains("Contexto Conceptual"));
    assert!(context_md.contains("1.1 Ramas Locales") || context_md.contains("git branch"));
}
