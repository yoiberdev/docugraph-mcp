use docugraph::document::model::{Document, DocumentMetadata, Page, SectionNode};
use docugraph::mcp::{DocuGraphServer, ToolResult, tools::*};
use docugraph::storage::DocumentStore;
use rmcp::handler::server::tool::IntoCallToolResult;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResponse;
use std::time::{Duration, Instant};

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
        .await
        .expect("tool call should succeed");

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
        .await
        .expect("tool call should succeed");

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
        .await
        .expect("tool call should succeed");

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
        .expect("tool call should succeed");

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
        .await
        .expect("tool call should succeed");

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
        .expect("tool call should succeed");

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
        .await
        .expect("tool call should succeed");

    assert!(context_md.contains("Contexto Conceptual"));
    assert!(context_md.contains("1.1 Ramas Locales") || context_md.contains("git branch"));
}

#[tokio::test]
async fn test_mcp_domain_errors_are_tool_errors() {
    let server = create_test_server();

    let err = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "missing-doc".to_string(),
        }))
        .await
        .expect_err("unknown document must be a tool error");
    assert!(err.message().contains("'missing-doc' not found"), "{err}");
    assert!(
        err.message().contains("'git-guide'"),
        "error should list the available ids: {err}"
    );

    let err = server
        .document_outline(Parameters(DocumentOutlineParams {
            document_id: "missing-doc".to_string(),
            max_depth: None,
        }))
        .await
        .expect_err("unknown document must be a tool error");
    assert!(err.message().contains("not found"), "{err}");

    let err = server
        .document_get_section(Parameters(DocumentGetSectionParams {
            document_id: "git-guide".to_string(),
            section_id: "no-such-section".to_string(),
            include_parent: None,
            max_tokens: None,
        }))
        .await
        .expect_err("unknown section must be a tool error");
    assert!(
        err.message()
            .contains("Section 'no-such-section' not found"),
        "{err}"
    );

    let err = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "missing-doc".to_string(),
            page_start: 1,
            page_end: 1,
            max_chars: None,
        }))
        .await
        .expect_err("unknown document must be a tool error");
    assert!(err.message().contains("not found"), "{err}");

    let err = server
        .document_render_page(Parameters(RenderPageParams {
            document_id: "git-guide".to_string(),
            page_number: 99,
            max_width: None,
        }))
        .await
        .expect_err("missing page must be a tool error");
    assert!(err.message().contains("Page 99 does not exist"), "{err}");

    let err = server
        .document_get_forms(Parameters(DocumentGetFormsParams {
            document_id: "git-guide".to_string(),
            page: Some(0),
            filled_only: None,
        }))
        .await
        .expect_err("page 0 must be a tool error");
    assert!(err.message().contains("Page 0 does not exist"), "{err}");

    let err = server
        .document_get_attachments(Parameters(DocumentGetAttachmentsParams {
            document_id: "missing-doc".to_string(),
        }))
        .await
        .expect_err("unknown document must be a tool error");
    assert!(err.message().contains("not found"), "{err}");

    let err = server
        .document_read_attachment(Parameters(DocumentReadAttachmentParams {
            document_id: "git-guide".to_string(),
            name_or_id: "invoice.xml".to_string(),
            max_bytes: None,
            encoding: None,
        }))
        .await
        .expect_err("unknown attachment must be a tool error");
    assert!(err.message().contains("has no attachments"), "{err}");
}

#[tokio::test]
async fn test_mcp_tool_error_is_sent_with_is_error() {
    let server = create_test_server();
    let result = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: "missing-doc".to_string(),
        }))
        .await;

    let response = result
        .into_call_tool_result()
        .expect("a domain error is a tool result, not a JSON-RPC error");
    let CallToolResponse::Complete(call_result) = response else {
        panic!("expected a complete tool result");
    };
    assert_eq!(call_result.is_error, Some(true));

    let wire = serde_json::to_value(&call_result).expect("serialize tool result");
    assert_eq!(wire["isError"], true);
    let text = wire["content"][0]["text"].as_str().expect("text content");
    assert!(text.contains("'missing-doc' not found"), "{text}");
}

#[tokio::test]
async fn test_mcp_search_tools_reject_unknown_document_id() {
    let server = create_test_server();
    let typo = Some("git-gide".to_string());

    let check = |tool: &str, result: ToolResult| {
        let err = result.expect_err(tool);
        assert!(
            err.message().contains("'git-gide' not found"),
            "{tool}: {err}"
        );
        assert!(
            err.message().contains("'git-guide'"),
            "{tool} should list the available ids: {err}"
        );
    };

    check(
        "document_search",
        server
            .document_search(Parameters(DocumentSearchParams {
                query: "ramas locales".to_string(),
                document_id: typo.clone(),
                limit: None,
            }))
            .await,
    );
    check(
        "document_search_hybrid",
        server
            .document_search_hybrid(Parameters(DocumentSearchHybridParams {
                query: "ramas locales".to_string(),
                document_id: typo.clone(),
                limit: None,
                bm25_weight: None,
                semantic_weight: None,
                structural_weight: None,
            }))
            .await,
    );
    check(
        "document_get_context",
        server
            .document_get_context(Parameters(DocumentGetContextParams {
                query: "ramas locales".to_string(),
                document_id: typo.clone(),
                max_tokens: None,
                max_chunks: None,
            }))
            .await,
    );
    check(
        "document_get_evidence",
        server
            .document_get_evidence(Parameters(DocumentGetEvidenceParams {
                query: "ramas locales".to_string(),
                document_id: typo,
                max_tokens: None,
                max_items: None,
            }))
            .await,
    );

    // Without document_id the search still covers every indexed document.
    let hits = server
        .document_search(Parameters(DocumentSearchParams {
            query: "ramas locales".to_string(),
            document_id: None,
            limit: Some(3),
        }))
        .await
        .expect("search without document_id");
    let hits: serde_json::Value = serde_json::from_str(&hits).expect("valid JSON hits");
    assert_eq!(hits[0]["document_id"], "git-guide");
}

#[test]
fn test_bounded_size_arguments() {
    assert_eq!(bounded(None, 5, MAX_SEARCH_LIMIT), 5);
    assert_eq!(bounded(Some(0), 5, MAX_SEARCH_LIMIT), 1);
    assert_eq!(bounded(Some(7), 5, MAX_SEARCH_LIMIT), 7);
    assert_eq!(
        bounded(Some(usize::MAX), 5, MAX_SEARCH_LIMIT),
        MAX_SEARCH_LIMIT
    );
}

#[tokio::test]
async fn test_mcp_read_pages_clamps_page_end() {
    let server = create_test_server();
    let started = Instant::now();
    let pages_md = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "git-guide".to_string(),
            page_start: 2,
            page_end: u32::MAX,
            max_chars: None,
        }))
        .await
        .expect("an open-ended range is clamped, not rejected");

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "page_end=u32::MAX took {:?}",
        started.elapsed()
    );
    assert!(pages_md.contains("(pp. 2-5)"), "{pages_md}");
    assert!(pages_md.contains("ajustado"), "{pages_md}");
    assert!(pages_md.contains("Página 3"), "{pages_md}");
}

#[tokio::test]
async fn test_mcp_read_pages_rejects_empty_ranges() {
    let server = create_test_server();
    let read = |page_start: u32, page_end: u32| {
        server.document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: "git-guide".to_string(),
            page_start,
            page_end,
            max_chars: None,
        }))
    };

    let err = read(0, 2).await.expect_err("page 0 does not exist");
    assert!(err.message().contains("at least 1"), "{err}");

    let err = read(3, 2).await.expect_err("inverted range");
    assert!(err.message().contains("Invalid page range 3-2"), "{err}");

    let err = read(6, u32::MAX)
        .await
        .expect_err("range that starts past the last page");
    assert!(err.message().contains("has 5 pages"), "{err}");

    // The fixture declares 5 pages but only pages 1-3 have text.
    let err = read(4, 5).await.expect_err("range without extracted pages");
    assert!(err.message().contains("is empty"), "{err}");
}

#[tokio::test]
async fn test_mcp_oversized_limits_are_clamped() {
    let server = create_test_server();

    let hits = server
        .document_search(Parameters(DocumentSearchParams {
            query: "ramas conflictos git".to_string(),
            document_id: None,
            limit: Some(usize::MAX),
        }))
        .await
        .expect("huge limit is clamped");
    let hits: serde_json::Value = serde_json::from_str(&hits).expect("valid JSON hits");
    assert!(hits.as_array().unwrap().len() <= MAX_SEARCH_LIMIT);

    let hits = server
        .document_search_hybrid(Parameters(DocumentSearchHybridParams {
            query: "ramas conflictos git".to_string(),
            document_id: None,
            limit: Some(usize::MAX),
            bm25_weight: None,
            semantic_weight: None,
            structural_weight: None,
        }))
        .await
        .expect("huge limit is clamped");
    let hits: serde_json::Value = serde_json::from_str(&hits).expect("valid JSON hits");
    assert!(hits.as_array().unwrap().len() <= MAX_SEARCH_LIMIT);

    let context = server
        .document_get_context(Parameters(DocumentGetContextParams {
            query: "ramas locales".to_string(),
            document_id: None,
            max_tokens: Some(usize::MAX),
            max_chunks: Some(usize::MAX),
        }))
        .await
        .expect("huge budgets are clamped");
    assert!(
        context.contains(&format!("~{MAX_CONTEXT_TOKENS} tokens")),
        "{context}"
    );

    let evidence = server
        .document_get_evidence(Parameters(DocumentGetEvidenceParams {
            query: "ramas locales".to_string(),
            document_id: None,
            max_tokens: Some(usize::MAX),
            max_items: Some(usize::MAX),
        }))
        .await
        .expect("huge budgets are clamped");
    assert!(evidence.contains("Evidencia Recuperada"), "{evidence}");

    let section = server
        .document_get_section(Parameters(DocumentGetSectionParams {
            document_id: "git-guide".to_string(),
            section_id: "ramas-locales".to_string(),
            include_parent: None,
            max_tokens: Some(usize::MAX),
        }))
        .await
        .expect("huge budget is clamped");
    assert!(section.contains("1.1 Ramas Locales"), "{section}");
}

#[tokio::test]
async fn test_mcp_get_section_with_broken_page_range_returns_quickly() {
    let store = DocumentStore::new(None);
    let mut doc = Document::new(DocumentMetadata {
        id: "broken-outline".to_string(),
        title: "Broken Outline".to_string(),
        total_pages: 2,
        content_hash: "broken-outline-hash".to_string(),
        ..Default::default()
    });
    doc.add_page(Page::new(1, "Primera página."));
    doc.add_page(Page::new(2, "Segunda página."));
    // A malformed outline can claim that a section runs until u32::MAX.
    doc.sections.push(SectionNode::new(
        "whole-book",
        "Whole Book",
        1,
        1,
        u32::MAX,
        None,
    ));
    store.insert(doc).expect("insert must succeed");
    let server = DocuGraphServer::with_store(store);

    let started = Instant::now();
    let content = server
        .document_get_section(Parameters(DocumentGetSectionParams {
            document_id: "broken-outline".to_string(),
            section_id: "whole-book".to_string(),
            include_parent: None,
            max_tokens: None,
        }))
        .await
        .expect("section exists");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "section with page_end=u32::MAX took {:?}",
        started.elapsed()
    );
    assert!(content.contains("Segunda página."), "{content}");
}
