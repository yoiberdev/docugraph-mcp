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
        .await
        .expect("outline must succeed for an indexed document");

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
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales trabajo".to_string(),
            document_id: Some("git-guide".to_string()),
            mode: Some(QueryMode::Hits),
            max_tokens: None,
            max_items: Some(3),
        }))
        .await
        .expect("search must succeed for an indexed document_id");

    let hits: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON hits");
    assert!(hits.is_array());
    assert!(!hits.as_array().unwrap().is_empty());
    assert_eq!(hits[0]["document_id"], "git-guide");
}

#[tokio::test]
async fn test_mcp_document_search_hybrid() {
    let server = create_test_server();
    let resp = server
        .document_query(Parameters(DocumentQueryParams {
            query: "fusión y resolución de conflictos".to_string(),
            document_id: None,
            mode: Some(QueryMode::Hits),
            max_tokens: None,
            max_items: Some(2),
        }))
        .await
        .expect("hybrid search must succeed over the whole corpus");

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
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales".to_string(),
            document_id: None,
            mode: Some(QueryMode::Evidence),
            max_tokens: Some(500),
            max_items: Some(2),
        }))
        .await
        .expect("evidence must succeed over the whole corpus");

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
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales de trabajo".to_string(),
            document_id: Some("git-guide".to_string()),
            mode: Some(QueryMode::Context),
            max_tokens: Some(600),
            max_items: Some(2),
        }))
        .await
        .expect("context must succeed for an indexed document_id");

    assert!(context_md.contains("Contexto Conceptual"));
    assert!(context_md.contains("1.1 Ramas Locales") || context_md.contains("git branch"));
}

/// An explicit but unresolvable `document_id` must be an error on every scoped tool.
///
/// Regression: these tools used to fall back to the whole corpus, so a typo returned
/// confidently-cited passages from a different document with no signal at all.
#[tokio::test]
async fn test_mcp_unknown_document_id_is_an_error_not_a_silent_corpus_wide_search() {
    let server = create_test_server();
    let unknown = "libro-inexistente-xyz".to_string();

    let search = server
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales".to_string(),
            document_id: Some(unknown.clone()),
            mode: Some(QueryMode::Hits),
            max_tokens: None,
            max_items: Some(3),
        }))
        .await;
    let err = search.expect_err("an unknown document_id must not resolve to the whole corpus");
    assert!(
        err.contains(&unknown),
        "the error must name the bad id: {err}"
    );
    assert!(
        err.contains("git-guide"),
        "the error must list the available ids so the agent can correct itself: {err}"
    );

    let hybrid = server
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales".to_string(),
            document_id: Some(unknown.clone()),
            mode: Some(QueryMode::Hits),
            max_tokens: None,
            max_items: Some(3),
        }))
        .await;
    assert!(hybrid.is_err(), "hybrid search must reject an unknown id");

    let evidence = server
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales".to_string(),
            document_id: Some(unknown.clone()),
            mode: Some(QueryMode::Evidence),
            max_tokens: Some(500),
            max_items: Some(2),
        }))
        .await;
    assert!(evidence.is_err(), "evidence must reject an unknown id");

    let context = server
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales".to_string(),
            document_id: Some(unknown),
            mode: Some(QueryMode::Context),
            max_tokens: Some(600),
            max_items: Some(2),
        }))
        .await;
    assert!(context.is_err(), "context must reject an unknown id");
}

/// A blank `document_id` is an unambiguous "no filter", not a typo: it must widen
/// to the whole corpus rather than error.
#[tokio::test]
async fn test_mcp_blank_document_id_is_treated_as_no_filter() {
    let server = create_test_server();
    let resp = server
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales".to_string(),
            document_id: Some("   ".to_string()),
            mode: Some(QueryMode::Hits),
            max_tokens: None,
            max_items: Some(3),
        }))
        .await
        .expect("a blank document_id must mean 'search everything'");

    let hits: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON hits");
    assert!(hits.is_array());
}

/// The store resolves a document by content hash even with no disk cache configured.
#[tokio::test]
async fn test_mcp_document_id_accepts_a_content_hash() {
    let server = create_test_server();
    let resp = server
        .document_query(Parameters(DocumentQueryParams {
            query: "ramas locales".to_string(),
            document_id: Some("hashgit123".to_string()),
            mode: Some(QueryMode::Hits),
            max_tokens: None,
            max_items: Some(3),
        }))
        .await
        .expect("a content hash is a valid document identifier");

    let hits: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON hits");
    assert!(hits.is_array());
}

/// Every tool that takes a `document_id` must report an unknown one as a tool
/// error, so the MCP client sees `isError` rather than a successful response.
///
/// Regression: `document_get_links`, `document_get_forms` and
/// `document_get_attachments` returned `Ok` with an `{"error": ...}` body, and
/// `document_outline` returned `Ok("Error: Document ... not found.")`. Worst of
/// all, `document_info` returned a structurally valid result with
/// `total_pages: 0` and `content_hash: ""`, which reads as an empty document
/// rather than a missing one.
#[tokio::test]
async fn test_mcp_unknown_document_is_an_error_on_every_scoped_tool() {
    let server = create_test_server();
    let unknown = "no-such-document".to_string();

    let info = server
        .document_info(Parameters(DocumentInfoParams {
            document_id: unknown.clone(),
        }))
        .await;
    let err = info.expect_err("document_info must not fabricate an empty document");
    assert!(err.contains(&unknown), "got: {err}");
    assert!(err.contains("git-guide"), "must list available ids: {err}");

    let outline = server
        .document_outline(Parameters(DocumentOutlineParams {
            document_id: unknown.clone(),
            max_depth: None,
        }))
        .await;
    assert!(
        outline.is_err(),
        "document_outline must reject an unknown id"
    );

    let links = server
        .document_get_links(Parameters(DocumentGetLinksParams {
            document_id: unknown.clone(),
            page: None,
            kind: None,
        }))
        .await;
    assert!(
        links.is_err(),
        "document_get_links must reject an unknown id"
    );

    let forms = server
        .document_get_forms(Parameters(DocumentGetFormsParams {
            document_id: unknown.clone(),
            page: None,
            filled_only: None,
        }))
        .await;
    assert!(
        forms.is_err(),
        "document_get_forms must reject an unknown id"
    );

    let attachments = server
        .document_get_attachments(Parameters(DocumentGetAttachmentsParams {
            document_id: unknown.clone(),
        }))
        .await;
    assert!(
        attachments.is_err(),
        "document_get_attachments must reject an unknown id"
    );

    let section = server
        .document_get_section(Parameters(DocumentGetSectionParams {
            document_id: unknown,
            section_id: "ramas-locales".to_string(),
            include_parent: None,
            max_tokens: None,
        }))
        .await;
    assert!(section.is_err());
}

/// A missing section names where to find the real ones, the same way a missing
/// document names the available document ids.
#[tokio::test]
async fn test_mcp_unknown_section_points_at_the_outline() {
    let server = create_test_server();
    let err = server
        .document_get_section(Parameters(DocumentGetSectionParams {
            document_id: "git-guide".to_string(),
            section_id: "no-such-section".to_string(),
            include_parent: None,
            max_tokens: None,
        }))
        .await
        .expect_err("an unknown section_id must be an error");

    assert!(err.contains("no-such-section"), "got: {err}");
    assert!(err.contains("document_outline"), "got: {err}");
}
