//! Citations must point at the page a snippet comes from, not at the first page of its section.

use docugraph::document::{Document, DocumentMetadata, Page, SectionNode};
use docugraph::mcp::{DocuGraphServer, DocumentGetEvidenceParams, DocumentSearchParams};
use docugraph::retrieval::SearchHit;
use docugraph::storage::DocumentStore;
use rmcp::handler::server::wrapper::Parameters;

/// Three-page guide covered by one section; the query term only appears on page 3.
fn guide_document() -> Document {
    let mut doc = Document::new(DocumentMetadata {
        id: "deploy-guide".to_string(),
        title: "Deployment Guide".to_string(),
        total_pages: 3,
        total_sections: 1,
        ..Default::default()
    });
    doc.add_page(Page::new(
        1,
        "Introduction to the deployment guide and its general scope.",
    ));
    doc.add_page(Page::new(
        2,
        "Configuration of servers, networks and storage volumes.",
    ));
    doc.add_page(Page::new(
        3,
        "Rollback procedure: restore the previous zanzibar release safely.",
    ));
    doc.sections.push(SectionNode::new(
        "deployment-guide-p1",
        "Deployment guide",
        1,
        1,
        3,
        None,
    ));
    doc
}

async fn server_with(doc: Document) -> DocuGraphServer {
    let server = DocuGraphServer::with_store(DocumentStore::new(None));
    server.register_document(doc).await;
    server
}

#[tokio::test]
async fn test_search_hit_reports_page_of_snippet() {
    let server = server_with(guide_document()).await;
    let json = server
        .document_search(Parameters(DocumentSearchParams {
            query: "zanzibar".to_string(),
            document_id: Some("deploy-guide".to_string()),
            limit: Some(5),
        }))
        .await;
    let hits: Vec<SearchHit> = serde_json::from_str(&json).expect("valid search JSON");

    let section = hits
        .iter()
        .find(|h| h.section_id.as_deref() == Some("deployment-guide-p1"))
        .expect("section hit");
    assert_eq!((section.page_start, section.page_end), (1, 3));
    assert_eq!(section.snippet_page, 3);
    assert!(hits.iter().all(|h| h.snippet_page == 3), "{json}");
}

#[tokio::test]
async fn test_evidence_cites_page_of_snippet_inside_section() {
    let server = server_with(guide_document()).await;
    let markdown = server
        .document_get_evidence(Parameters(DocumentGetEvidenceParams {
            query: "zanzibar".to_string(),
            document_id: Some("deploy-guide".to_string()),
            max_tokens: None,
            max_items: None,
        }))
        .await;

    let citations: Vec<&str> = markdown
        .lines()
        .filter(|line| line.starts_with("**["))
        .collect();
    let section_citation = citations
        .iter()
        .find(|c| c.contains("§ deployment-guide-p1"))
        .unwrap_or_else(|| panic!("section citation missing:\n{markdown}"));
    assert!(
        section_citation.contains("p. 3 §"),
        "section hit must cite the snippet page, got: {section_citation}"
    );
    assert!(
        citations
            .iter()
            .any(|c| c.contains("[Doc: deploy-guide p. 3]")),
        "{markdown}"
    );
}
