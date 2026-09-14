//! Citations must point at the page a snippet comes from, not at the first page of its section,
//! and show the printed page label next to the PDF page when the document defines one.

use docugraph::document::{
    Document, DocumentMetadata, Page, PageLabelRange, PageLabelStyle, SectionNode,
    format_page_number, page_labels::labels_for_ranges, printed_label_suffix,
};
use docugraph::mcp::{
    DocuGraphServer, DocumentGetEvidenceParams, DocumentInfoParams, DocumentInfoResult,
    DocumentOutlineParams, DocumentReadPagesParams, DocumentSearchParams, OutlineNodeResult,
};
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

#[test]
fn test_page_label_number_formats() {
    assert_eq!(
        format_page_number(1994, PageLabelStyle::UpperRoman),
        "MCMXCIV"
    );
    assert_eq!(format_page_number(4, PageLabelStyle::LowerRoman), "iv");
    assert_eq!(format_page_number(27, PageLabelStyle::UpperLetters), "AA");
    assert_eq!(format_page_number(53, PageLabelStyle::LowerLetters), "aaa");
    assert_eq!(format_page_number(12, PageLabelStyle::Decimal), "12");
    // Out of range for roman numerals: decimal instead of a wrong numeral
    assert_eq!(format_page_number(5000, PageLabelStyle::UpperRoman), "5000");

    assert_eq!(printed_label_suffix(9, Some("6")), " (impresa 6)");
    assert_eq!(printed_label_suffix(9, Some("9")), "");
    assert_eq!(printed_label_suffix(9, None), "");
}

#[test]
fn test_page_label_ranges_expand_per_page() {
    let range = |start_index, style, prefix: &str, first_number| PageLabelRange {
        start_index,
        style,
        prefix: prefix.to_string(),
        first_number,
    };
    let ranges = vec![
        range(0, Some(PageLabelStyle::LowerRoman), "", 1),
        range(2, None, "", 1),
        range(3, Some(PageLabelStyle::Decimal), "", 1),
        range(5, Some(PageLabelStyle::Decimal), "A-", 7),
    ];
    assert_eq!(
        labels_for_ranges(&ranges, 7),
        vec!["i", "ii", "", "1", "2", "A-7", "A-8"]
    );
}

/// The guide with printed labels that differ from the PDF page numbers.
fn labelled_guide() -> Document {
    let mut doc = guide_document();
    for page in &mut doc.pages {
        page.label = Some(format!("A-{}", page.page_number));
    }
    doc
}

#[tokio::test]
async fn test_page_labels_shown_in_info_outline_pages_and_evidence() {
    let server = server_with(labelled_guide()).await;
    let doc_id = "deploy-guide".to_string();

    let info: DocumentInfoResult = serde_json::from_str(
        &server
            .document_info(Parameters(DocumentInfoParams {
                document_id: doc_id.clone(),
            }))
            .await,
    )
    .expect("valid info JSON");
    assert!(info.has_page_labels);
    assert_eq!(
        info.sections_preview,
        vec!["* Deployment guide (pp. 1-3, impresas A-1-A-3)"]
    );

    let outline: Vec<OutlineNodeResult> = serde_json::from_str(
        &server
            .document_outline(Parameters(DocumentOutlineParams {
                document_id: doc_id.clone(),
                max_depth: None,
            }))
            .await,
    )
    .expect("valid outline JSON");
    assert_eq!(outline[0].page_label_start.as_deref(), Some("A-1"));
    assert_eq!(outline[0].page_label_end.as_deref(), Some("A-3"));

    let pages = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: doc_id.clone(),
            page_start: 3,
            page_end: 3,
            max_chars: None,
        }))
        .await;
    assert!(pages.contains("--- Página 3 (impresa A-3) ---"), "{pages}");

    let evidence = server
        .document_get_evidence(Parameters(DocumentGetEvidenceParams {
            query: "zanzibar".to_string(),
            document_id: Some(doc_id),
            max_tokens: None,
            max_items: None,
        }))
        .await;
    assert!(
        evidence.contains("p. 3 (impresa A-3) § deployment-guide-p1"),
        "{evidence}"
    );
    assert!(
        evidence.contains("[Doc: deploy-guide p. 3 (impresa A-3)]"),
        "{evidence}"
    );
}

#[tokio::test]
async fn test_no_page_labels_means_no_printed_numbers() {
    let server = server_with(guide_document()).await;
    let doc_id = "deploy-guide".to_string();

    let info: DocumentInfoResult = serde_json::from_str(
        &server
            .document_info(Parameters(DocumentInfoParams {
                document_id: doc_id.clone(),
            }))
            .await,
    )
    .expect("valid info JSON");
    assert!(!info.has_page_labels);
    assert_eq!(info.sections_preview, vec!["* Deployment guide (pp. 1-3)"]);

    let outline_json = server
        .document_outline(Parameters(DocumentOutlineParams {
            document_id: doc_id.clone(),
            max_depth: None,
        }))
        .await;
    assert!(!outline_json.contains("page_label"), "{outline_json}");

    let pages = server
        .document_read_pages(Parameters(DocumentReadPagesParams {
            document_id: doc_id,
            page_start: 3,
            page_end: 3,
            max_chars: None,
        }))
        .await;
    assert!(pages.contains("--- Página 3 ---"), "{pages}");
}
