//! Tests for Milestone 7: Hyperlink extraction, internal cross-references, and outline strategy.

use docugraph::document::{
    Document, DocumentLink, DocumentMetadata, FallbackOutlineStrategy, LinkTarget,
    NativeOutlineExtractor, OutlineExtractor, Page, TypographicOutlineExtractor,
    extract_page_links,
};
use docugraph::mcp::{DocuGraphServer, DocumentGetLinksParams, DocumentGetLinksResult};
use docugraph::storage::DocumentStore;
use lopdf::{Dictionary, Document as LopdfDoc, Object};
use rmcp::handler::server::wrapper::Parameters;
use std::collections::HashMap;

type TestDocFixture = (LopdfDoc, (u32, u16), (u32, u16), HashMap<(u32, u16), u32>);

/// Helper to create a minimal lopdf::Document with 2 pages for testing.
fn create_test_lopdf_doc() -> TestDocFixture {
    let mut doc = LopdfDoc::with_version("1.7");

    let pages_id = doc.new_object_id();
    let page1_id = doc.new_object_id();
    let page2_id = doc.new_object_id();

    let mut page1_dict = Dictionary::new();
    page1_dict.set("Type", Object::Name(b"Page".to_vec()));
    page1_dict.set("Parent", Object::Reference(pages_id));

    let mut page2_dict = Dictionary::new();
    page2_dict.set("Type", Object::Name(b"Page".to_vec()));
    page2_dict.set("Parent", Object::Reference(pages_id));

    doc.objects.insert(page1_id, Object::Dictionary(page1_dict));
    doc.objects.insert(page2_id, Object::Dictionary(page2_dict));

    let mut pages_dict = Dictionary::new();
    pages_dict.set("Type", Object::Name(b"Pages".to_vec()));
    pages_dict.set("Count", Object::Integer(2));
    pages_dict.set(
        "Kids",
        Object::Array(vec![
            Object::Reference(page1_id),
            Object::Reference(page2_id),
        ]),
    );
    doc.objects.insert(pages_id, Object::Dictionary(pages_dict));

    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = doc.add_object(Object::Dictionary(catalog));

    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut page_map = HashMap::new();
    page_map.insert(page1_id, 1);
    page_map.insert(page2_id, 2);

    (doc, page1_id, page2_id, page_map)
}

#[test]
fn test_extract_external_uri_link() {
    let (mut doc, page1_id, _, page_map) = create_test_lopdf_doc();

    // Create an external URI link annotation
    let mut action_dict = Dictionary::new();
    action_dict.set("S", Object::Name(b"URI".to_vec()));
    action_dict.set(
        "URI",
        Object::String(
            b"https://docugraph.io/spec".to_vec(),
            lopdf::StringFormat::Literal,
        ),
    );

    let mut annot_dict = Dictionary::new();
    annot_dict.set("Type", Object::Name(b"Annot".to_vec()));
    annot_dict.set("Subtype", Object::Name(b"Link".to_vec()));
    annot_dict.set(
        "Rect",
        Object::Array(vec![
            Object::Real(72.0),
            Object::Real(100.0),
            Object::Real(250.0),
            Object::Real(120.0),
        ]),
    );
    annot_dict.set("A", Object::Dictionary(action_dict));

    let annot_id = doc.add_object(Object::Dictionary(annot_dict));

    // Attach annotation to page 1
    if let Some(Object::Dictionary(p_dict)) = doc.objects.get_mut(&page1_id) {
        p_dict.set("Annots", Object::Array(vec![Object::Reference(annot_id)]));
    }

    let p1_dict = doc.get_dictionary(page1_id).unwrap();
    let links = extract_page_links(&doc, p1_dict, 1, &page_map);

    assert_eq!(links.len(), 1, "Should extract 1 external link");
    let link = &links[0];
    assert_eq!(link.page_number, 1);
    assert!(link.is_external());
    assert!(!link.is_internal());
    assert_eq!(link.uri.as_deref(), Some("https://docugraph.io/spec"));
    assert_eq!(link.target_page, None);
    assert_eq!(link.rect, Some([72.0, 100.0, 250.0, 120.0]));
}

#[test]
fn test_extract_internal_goto_link_with_page_resolution() {
    let (mut doc, page1_id, page2_id, page_map) = create_test_lopdf_doc();

    // Create an internal GoTo link annotation pointing to page 2
    let mut action_dict = Dictionary::new();
    action_dict.set("S", Object::Name(b"GoTo".to_vec()));
    action_dict.set(
        "D",
        Object::Array(vec![
            Object::Reference(page2_id),
            Object::Name(b"XYZ".to_vec()),
            Object::Integer(0),
            Object::Integer(700),
            Object::Integer(1),
        ]),
    );

    let mut annot_dict = Dictionary::new();
    annot_dict.set("Type", Object::Name(b"Annot".to_vec()));
    annot_dict.set("Subtype", Object::Name(b"Link".to_vec()));
    annot_dict.set("A", Object::Dictionary(action_dict));
    annot_dict.set(
        "Rect",
        Object::Array(vec![
            Object::Integer(50),
            Object::Integer(80),
            Object::Integer(180),
            Object::Integer(95),
        ]),
    );

    let annot_id = doc.add_object(Object::Dictionary(annot_dict));

    if let Some(Object::Dictionary(p_dict)) = doc.objects.get_mut(&page1_id) {
        p_dict.set("Annots", Object::Array(vec![Object::Reference(annot_id)]));
    }

    let p1_dict = doc.get_dictionary(page1_id).unwrap();
    let links = extract_page_links(&doc, p1_dict, 1, &page_map);

    assert_eq!(links.len(), 1, "Should extract 1 internal GoTo link");
    let link = &links[0];
    assert_eq!(link.page_number, 1);
    assert!(link.is_internal());
    assert!(!link.is_external());
    assert_eq!(link.target_page, Some(2), "Must resolve target page 2");
    assert_eq!(link.uri, None);
    assert_eq!(link.rect, Some([50.0, 80.0, 180.0, 95.0]));
}

#[test]
fn test_extract_internal_link_direct_dest() {
    let (mut doc, page1_id, page2_id, page_map) = create_test_lopdf_doc();

    // PDF spec allows /Dest directly on /Link annot without /A
    let mut annot_dict = Dictionary::new();
    annot_dict.set("Subtype", Object::Name(b"Link".to_vec()));
    annot_dict.set(
        "Dest",
        Object::Array(vec![
            Object::Reference(page2_id),
            Object::Name(b"Fit".to_vec()),
        ]),
    );

    let annot_id = doc.add_object(Object::Dictionary(annot_dict));

    if let Some(Object::Dictionary(p_dict)) = doc.objects.get_mut(&page1_id) {
        p_dict.set("Annots", Object::Array(vec![Object::Reference(annot_id)]));
    }

    let p1_dict = doc.get_dictionary(page1_id).unwrap();
    let links = extract_page_links(&doc, p1_dict, 1, &page_map);

    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, LinkTarget::InternalPage(2));
    assert_eq!(links[0].target_page, Some(2));
}

#[test]
fn test_outline_strategy_fallback_on_document_without_outlines() {
    let (doc, _, _, page_map) = create_test_lopdf_doc();

    let pages = vec![
        Page::new(
            1,
            "Chapter 1: Introduction to Architecture\nGeneral overview of system components.",
        ),
        Page::new(
            2,
            "Chapter 2: Structural Design Patterns\nAdapter and Composite implementations.",
        ),
    ];

    let strategy = FallbackOutlineStrategy::new();
    let sections = strategy.extract(&doc, &page_map, &pages, 2);

    assert!(
        !sections.is_empty(),
        "Fallback strategy must infer sections"
    );
    assert_eq!(sections.len(), 2, "Should infer 2 root chapter sections");
    assert!(sections[0].title.contains("Chapter 1"));
    assert_eq!(sections[0].page_start, 1);
    assert!(sections[1].title.contains("Chapter 2"));
    assert_eq!(sections[1].page_start, 2);
}

#[test]
fn test_outline_strategy_native_outlines_preferred() {
    let (mut doc, page1_id, page2_id, page_map) = create_test_lopdf_doc();

    // Build native /Outlines structure
    let item2_id = doc.new_object_id();
    let mut item2_dict = Dictionary::new();
    item2_dict.set(
        "Title",
        Object::String(
            b"2. Behavioral Patterns".to_vec(),
            lopdf::StringFormat::Literal,
        ),
    );
    item2_dict.set(
        "Dest",
        Object::Array(vec![
            Object::Reference(page2_id),
            Object::Name(b"Fit".to_vec()),
        ]),
    );
    doc.objects.insert(item2_id, Object::Dictionary(item2_dict));

    let item1_id = doc.new_object_id();
    let mut item1_dict = Dictionary::new();
    item1_dict.set(
        "Title",
        Object::String(
            b"1. Creational Patterns".to_vec(),
            lopdf::StringFormat::Literal,
        ),
    );
    item1_dict.set(
        "Dest",
        Object::Array(vec![
            Object::Reference(page1_id),
            Object::Name(b"Fit".to_vec()),
        ]),
    );
    item1_dict.set("Next", Object::Reference(item2_id));
    doc.objects.insert(item1_id, Object::Dictionary(item1_dict));

    let outlines_id = doc.new_object_id();
    let mut outlines_dict = Dictionary::new();
    outlines_dict.set("Type", Object::Name(b"Outlines".to_vec()));
    outlines_dict.set("First", Object::Reference(item1_id));
    doc.objects
        .insert(outlines_id, Object::Dictionary(outlines_dict));

    // Hook to Catalog /Root
    let root_ref = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    if let Some(Object::Dictionary(catalog)) = doc.objects.get_mut(&root_ref) {
        catalog.set("Outlines", Object::Reference(outlines_id));
    }

    let pages = vec![
        Page::new(1, "Text on page 1."),
        Page::new(2, "Text on page 2."),
    ];

    let strategy = FallbackOutlineStrategy::new();
    let sections = strategy.extract(&doc, &page_map, &pages, 2);

    assert_eq!(sections.len(), 2, "Must extract 2 native outline sections");
    assert_eq!(sections[0].title, "1. Creational Patterns");
    assert_eq!(sections[0].page_start, 1);
    assert_eq!(sections[1].title, "2. Behavioral Patterns");
    assert_eq!(sections[1].page_start, 2);
}

#[test]
fn test_typographic_outline_extractor_direct() {
    let (doc, _, _, page_map) = create_test_lopdf_doc();
    let pages = vec![
        Page::new(1, "1. Introduction\nContent here."),
        Page::new(2, "2. Architecture\nContent here."),
    ];
    let extractor = TypographicOutlineExtractor;
    let sections = extractor.extract(&doc, &page_map, &pages, 2);
    assert_eq!(sections.len(), 2);
}

#[test]
fn test_native_outline_extractor_empty_on_plain_doc() {
    let (doc, _, _, page_map) = create_test_lopdf_doc();
    let pages = vec![Page::new(1, "No outlines")];
    let extractor = NativeOutlineExtractor;
    let sections = extractor.extract(&doc, &page_map, &pages, 1);
    assert!(sections.is_empty());
}

#[tokio::test]
async fn test_mcp_tool_document_get_links() {
    let store = DocumentStore::new(None);
    let server = DocuGraphServer::with_store(store);

    let mut doc = Document::new(DocumentMetadata {
        id: "links-demo-doc".to_string(),
        title: "Hyperlink Reference Manual".to_string(),
        author: Some("DocuGraph Team".to_string()),
        total_pages: 3,
        total_sections: 1,
        file_size_bytes: 4096,
        content_hash: "linkhash999".to_string(),
        indexed_at: "2026-09-14T00:00:00Z".to_string(),
        is_encrypted: false,
        untrusted_text_detected: false,
        scanned_pages_count: 0,
        source_path: None,
        total_links: 3,
    });

    let mut p1 = Page::new(1, "Page 1 with external link and cross-reference.");
    p1.links.push(DocumentLink::uri(
        1,
        "https://github.com/yoiber/docugraph",
        Some([10.0, 20.0, 100.0, 30.0]),
    ));
    p1.links
        .push(DocumentLink::internal(1, 3, Some([10.0, 40.0, 80.0, 50.0])));
    doc.add_page(p1);

    let mut p2 = Page::new(2, "Page 2 with named link.");
    p2.links.push(DocumentLink::named(2, "appendix-b", None));
    doc.add_page(p2);

    let p3 = Page::new(3, "Page 3 target destination.");
    doc.add_page(p3);

    server.register_document(doc).await;

    // 1. Query all links
    let all_links_json = server
        .document_get_links(Parameters(DocumentGetLinksParams {
            document_id: "links-demo-doc".to_string(),
            page: None,
            kind: Some("all".to_string()),
        }))
        .await;

    let res: DocumentGetLinksResult =
        serde_json::from_str(&all_links_json).expect("valid JSON result");
    assert_eq!(res.document_id, "links-demo-doc");
    assert_eq!(res.total_links, 3);
    assert_eq!(res.links.len(), 3);

    // 2. Query only external links
    let ext_json = server
        .document_get_links(Parameters(DocumentGetLinksParams {
            document_id: "links-demo-doc".to_string(),
            page: None,
            kind: Some("external".to_string()),
        }))
        .await;
    let ext_res: DocumentGetLinksResult =
        serde_json::from_str(&ext_json).expect("valid JSON result");
    assert_eq!(ext_res.total_links, 1);
    assert_eq!(ext_res.links[0].kind, "external");
    assert_eq!(
        ext_res.links[0].uri.as_deref(),
        Some("https://github.com/yoiber/docugraph")
    );

    // 3. Query only internal links
    let int_json = server
        .document_get_links(Parameters(DocumentGetLinksParams {
            document_id: "links-demo-doc".to_string(),
            page: None,
            kind: Some("internal".to_string()),
        }))
        .await;
    let int_res: DocumentGetLinksResult =
        serde_json::from_str(&int_json).expect("valid JSON result");
    assert_eq!(int_res.total_links, 1);
    assert_eq!(int_res.links[0].kind, "internal");
    assert_eq!(int_res.links[0].target_page, Some(3));

    // 4. Query links by page
    let p2_json = server
        .document_get_links(Parameters(DocumentGetLinksParams {
            document_id: "links-demo-doc".to_string(),
            page: Some(2),
            kind: None,
        }))
        .await;
    let p2_res: DocumentGetLinksResult = serde_json::from_str(&p2_json).expect("valid JSON result");
    assert_eq!(p2_res.total_links, 1);
    assert_eq!(p2_res.links[0].page_number, 2);
    assert_eq!(p2_res.links[0].kind, "named");
    assert_eq!(p2_res.links[0].named_target.as_deref(), Some("appendix-b"));

    // 5. Query non-existent document
    let not_found_json = server
        .document_get_links(Parameters(DocumentGetLinksParams {
            document_id: "non-existent".to_string(),
            page: None,
            kind: None,
        }))
        .await;
    assert!(not_found_json.contains("not found"));
}
